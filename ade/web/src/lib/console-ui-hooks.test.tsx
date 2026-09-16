// @vitest-environment jsdom

import {
  useContainerNarrow,
  useCopyFlash,
  usePaneState,
  useWorkerLive,
} from '@iii-dev/console-ui/hooks'
import { act } from 'react'
import { createRoot } from 'react-dom/client'
import { afterEach, describe, expect, it, vi } from 'vitest'
import type { ExtensionIii } from '@/types/injectable-ui'

;(
  globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }
).IS_REACT_ACT_ENVIRONMENT = true

const mounted: Array<() => void> = []

function renderHook<T>(use: () => T) {
  const container = document.createElement('div')
  document.body.appendChild(container)
  const root = createRoot(container)
  let latest: T | undefined
  function Probe() {
    latest = use()
    return null
  }
  act(() => root.render(<Probe />))
  const unmount = () => {
    act(() => root.unmount())
    container.remove()
  }
  mounted.push(unmount)
  return {
    get current() {
      return latest as T
    },
    unmount,
  }
}

afterEach(() => {
  vi.unstubAllGlobals()
  for (const unmount of mounted.splice(0)) unmount()
  vi.useRealTimers()
  window.localStorage.clear()
})

describe('useContainerNarrow', () => {
  it('measures on attach, follows resizes, ignores zero widths', () => {
    const observers: Array<(entries: unknown[]) => void> = []
    const disconnect = vi.fn()
    vi.stubGlobal(
      'ResizeObserver',
      class {
        constructor(cb: (entries: unknown[]) => void) {
          observers.push(cb)
        }
        observe() {}
        disconnect = disconnect
      },
    )
    const hook = renderHook(() => useContainerNarrow({ below: 600 }))
    const node = document.createElement('div')
    node.getBoundingClientRect = () => ({ width: 400 }) as DOMRect
    act(() => hook.current.ref(node))
    expect(hook.current.narrow).toBe(true)

    act(() => observers[0]?.([{ contentRect: { width: 900 } }]))
    expect(hook.current.narrow).toBe(false)
    act(() => observers[0]?.([{ contentRect: { width: 0 } }]))
    expect(hook.current.narrow).toBe(false)

    act(() => hook.current.ref(null))
    expect(disconnect).toHaveBeenCalled()
  })

  it('survives a runtime without ResizeObserver', () => {
    vi.stubGlobal('ResizeObserver', undefined)
    const hook = renderHook(() => useContainerNarrow())
    const node = document.createElement('div')
    node.getBoundingClientRect = () => ({ width: 300 }) as DOMRect
    act(() => hook.current.ref(node))
    expect(hook.current.narrow).toBe(true)
  })
})

describe('usePaneState', () => {
  it('reads, updates (with an updater) and persists as JSON', () => {
    window.localStorage.setItem('pane', JSON.stringify({ open: true }))
    const hook = renderHook(() => usePaneState('pane', { open: false }))
    expect(hook.current[0]).toEqual({ open: true })
    act(() => hook.current[1]((prev) => ({ open: !prev.open })))
    expect(hook.current[0]).toEqual({ open: false })
    expect(window.localStorage.getItem('pane')).toBe('{"open":false}')
  })

  it('falls back to the initial value when storage is unusable', () => {
    window.localStorage.setItem('pane', '{not json')
    const setItem = vi
      .spyOn(Storage.prototype, 'setItem')
      .mockImplementation(() => {
        throw new Error('quota')
      })
    const hook = renderHook(() => usePaneState('pane', 1))
    expect(hook.current[0]).toBe(1)
    act(() => hook.current[1](2))
    expect(hook.current[0]).toBe(2)
    setItem.mockRestore()
  })
})

describe('useCopyFlash', () => {
  it('flashes copied, extends on a rapid second click, then idles', async () => {
    vi.useFakeTimers()
    const writeText = vi.fn().mockResolvedValue(undefined)
    vi.stubGlobal('navigator', { clipboard: { writeText } })
    const hook = renderHook(() => useCopyFlash('abc', 1000))
    expect(hook.current.state).toBe('idle')
    await act(async () => {
      hook.current.copy()
      await Promise.resolve()
    })
    expect(hook.current.state).toBe('copied')
    await act(async () => {
      vi.advanceTimersByTime(800)
      hook.current.copy()
      await Promise.resolve()
    })
    act(() => vi.advanceTimersByTime(800))
    expect(hook.current.state).toBe('copied')
    act(() => vi.advanceTimersByTime(300))
    expect(hook.current.state).toBe('idle')
    expect(writeText).toHaveBeenCalledTimes(2)
  })

  it('reports failure', async () => {
    const hook = renderHook(() => useCopyFlash('abc'))
    // No clipboard API and no document for the textarea fallback.
    vi.stubGlobal('navigator', {})
    vi.stubGlobal('document', undefined)
    await act(async () => {
      hook.current.copy()
      await Promise.resolve()
    })
    expect(hook.current.state).toBe('failed')
  })
})

function fakeIii(opts: { failRegister?: boolean } = {}) {
  const handlers = new Map<string, (payload: unknown) => void>()
  const offs: Array<ReturnType<typeof vi.fn>> = []
  const registered: string[] = []
  const iii = {
    browserId: 'console-1',
    trigger: vi.fn(),
    on: vi.fn((id: string, handler: (payload: unknown) => void) => {
      handlers.set(id, handler)
      const off = vi.fn()
      offs.push(off)
      return off
    }),
    registerTrigger: vi.fn((input: { type: string; function_id: string }) => {
      if (opts.failRegister) throw new Error('no such trigger')
      registered.push(`${input.type}→${input.function_id}`)
      const off = vi.fn()
      offs.push(off)
      return off
    }),
    addConnectionStateListener: vi.fn(() => () => {}),
  } as unknown as ExtensionIii
  return { iii, handlers, offs, registered }
}

describe('useWorkerLive', () => {
  it('fetches, binds one handler plus one trigger per type, refetches on events, unbinds', async () => {
    const { iii, handlers, offs, registered } = fakeIii()
    let calls = 0
    const fetch = vi.fn(async () => ({ n: ++calls }))
    const hook = renderHook(() =>
      useWorkerLive({
        iii,
        triggers: ['worktree.created', 'worktree.removed'],
        fetch,
        handlerId: 'iii::wt-ui::events',
      }),
    )
    expect(hook.current.loading).toBe(true)
    await act(async () => {})
    expect(hook.current.loading).toBe(false)
    expect(hook.current.data).toEqual({ n: 1 })
    expect(hook.current.error).toBeNull()
    expect(hook.current.live).toBe(true)
    expect(registered).toEqual([
      'worktree.created→iii::wt-ui::events::console-1',
      'worktree.removed→iii::wt-ui::events::console-1',
    ])

    await act(async () => {
      handlers.get('iii::wt-ui::events')?.({})
    })
    expect(hook.current.data).toEqual({ n: 2 })

    hook.unmount()
    for (const off of offs) expect(off).toHaveBeenCalledTimes(1)
  })

  it('drops stale responses', async () => {
    const { iii, handlers } = fakeIii()
    const resolvers: Array<(v: string) => void> = []
    const fetch = vi.fn(
      () =>
        new Promise<string>((resolve) => {
          resolvers.push(resolve)
        }),
    )
    const hook = renderHook(() =>
      useWorkerLive({ iii, triggers: ['t'], fetch, handlerId: 'h' }),
    )
    await act(async () => {
      handlers.get('h')?.({})
    })
    expect(resolvers).toHaveLength(2)
    await act(async () => {
      resolvers[1]?.('second')
    })
    await act(async () => {
      resolvers[0]?.('first')
    })
    expect(hook.current.data).toBe('second')
  })

  it('polls while not live and surfaces fetch errors', async () => {
    vi.useFakeTimers()
    const { iii } = fakeIii({ failRegister: true })
    const fetch = vi.fn().mockRejectedValue({ code: 'E1', message: 'down' })
    const hook = renderHook(() =>
      useWorkerLive({
        iii,
        triggers: ['t'],
        fetch,
        handlerId: 'h',
        pollMs: 500,
      }),
    )
    await act(async () => {})
    expect(hook.current.live).toBe(false)
    expect(hook.current.error).toBe('E1: down')
    await act(async () => {
      vi.advanceTimersByTime(500)
    })
    expect(fetch).toHaveBeenCalledTimes(2)
  })
})
