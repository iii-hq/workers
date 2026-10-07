// @vitest-environment jsdom
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { getIiiClient } from '@/lib/iii-client'
import {
  fetchWorkspaceLayout,
  setWorkspaceLayout,
  type WorkspaceLayoutValue,
} from '@/lib/workspace-layout'
import {
  type UseWorkspaceTabsReturn,
  useWorkspaceTabs,
} from './use-workspace-tabs'

vi.mock('@/lib/iii-client', () => ({ getIiiClient: vi.fn() }))
vi.mock('@/lib/workspace-layout', async (original) => ({
  ...(await original<typeof import('@/lib/workspace-layout')>()),
  fetchWorkspaceLayout: vi.fn(),
  setWorkspaceLayout: vi.fn(),
}))

const RING_FN = 'iii::console::workspace_changed'

let api: UseWorkspaceTabsReturn
let root: Root
let host: HTMLDivElement
let server: WorkspaceLayoutValue
let handlers: Map<string, () => void>
let bindings: { type: string }[]

const home = (screens: string[], name?: string): WorkspaceLayoutValue => ({
  tabs: [
    { id: 'tab-home', columns: screens.length, screens, ...(name && { name }) },
  ],
  activeTabId: 'tab-home',
})

const client = () => ({
  browserId: 'browser-1',
  on: (id: string, handler: () => void) => {
    handlers.set(id, handler)
    return () => handlers.delete(id)
  },
  registerTrigger: (input: { type: string }) => {
    bindings.push(input)
    return () => {}
  },
})

function Probe() {
  api = useWorkspaceTabs()
  return null
}

async function mount() {
  const qc = new QueryClient()
  await act(async () =>
    root.render(
      <QueryClientProvider client={qc}>
        <Probe />
      </QueryClientProvider>,
    ),
  )
}

/** Let promise chains and React Query's timer-scheduled notifications run. */
async function settle(ms = 20) {
  await act(async () => {
    if (vi.isFakeTimers()) await vi.advanceTimersByTimeAsync(ms)
    else await new Promise((done) => setTimeout(done, ms))
  })
}

async function ring() {
  await act(async () => handlers.get(RING_FN)?.())
}

beforeEach(() => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  vi.clearAllMocks()
  handlers = new Map()
  bindings = []
  server = home(['chat'])
  host = document.createElement('div')
  document.body.append(host)
  root = createRoot(host)
  vi.mocked(getIiiClient).mockResolvedValue(client() as never)
  vi.mocked(fetchWorkspaceLayout).mockImplementation(async () =>
    structuredClone(server),
  )
  vi.mocked(setWorkspaceLayout).mockImplementation(async (value) => {
    server = structuredClone(value)
  })
})

afterEach(async () => {
  await act(async () => root.unmount())
  host.remove()
  vi.useRealTimers()
  vi.restoreAllMocks()
  vi.unstubAllGlobals()
})

describe('workspace layout ring', () => {
  it('re-reads a ring that lands while a local write waits for its ack', async () => {
    let ack!: () => void
    vi.mocked(setWorkspaceLayout).mockImplementation(async (value) => {
      server = structuredClone(value)
      await new Promise<void>((done) => {
        ack = done
      })
    })
    await mount()
    await settle()
    expect(api.tabs[0].screens).toEqual(['chat'])

    await act(async () => api.renameTab('tab-home', 'Main'))
    await vi.waitFor(() => expect(setWorkspaceLayout).toHaveBeenCalled())
    // An agent opens traces after our write was stored, before its ack.
    server = home(['chat', 'traces'], 'Main')
    await ring()
    await act(async () => ack())
    await settle(50)

    expect(api.tabs[0].screens).toEqual(['chat', 'traces'])
    expect(api.tabs[0].name).toBe('Main')
  })

  it('binds again after the client bootstrap fails', async () => {
    vi.useFakeTimers()
    vi.mocked(getIiiClient).mockRejectedValueOnce(new Error('runtime failed'))
    await mount()
    await settle()
    expect(bindings).toHaveLength(0)

    await settle(1_000)
    expect(bindings).toEqual([
      expect.objectContaining({ type: 'console::workspace::changed' }),
    ])
  })

  it('never polls: it re-reads only on rings', async () => {
    vi.useFakeTimers()
    await mount()
    await settle()
    expect(fetchWorkspaceLayout).toHaveBeenCalledTimes(1)

    // No server copy yet and no ring: nothing re-reads on a timer.
    await settle(15_000)
    expect(fetchWorkspaceLayout).toHaveBeenCalledTimes(1)

    // The console rings every binding once when it lands: a catch-up read.
    server = home(['chat', 'traces'])
    await ring()
    await settle()
    expect(fetchWorkspaceLayout).toHaveBeenCalledTimes(2)
    expect(api.tabs[0].screens).toEqual(['chat', 'traces'])

    await settle(15_000)
    expect(fetchWorkspaceLayout).toHaveBeenCalledTimes(2)
  })
})
