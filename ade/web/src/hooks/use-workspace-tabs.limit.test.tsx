// @vitest-environment jsdom
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { getIiiClient } from '@/lib/iii-client'
import { DEFAULT_TAB_LIMIT } from '@/lib/tab-limit'
import {
  fetchWorkspaceLayout,
  setWorkspaceLayout,
  type WorkspaceLayoutValue,
} from '@/lib/workspace-layout'
import {
  type UseWorkspaceTabsOptions,
  type UseWorkspaceTabsReturn,
  useWorkspaceTabs,
} from './use-workspace-tabs'

vi.mock('@/lib/iii-client', () => ({ getIiiClient: vi.fn() }))
vi.mock('@/lib/workspace-layout', async (original) => ({
  ...(await original<typeof import('@/lib/workspace-layout')>()),
  fetchWorkspaceLayout: vi.fn(),
  setWorkspaceLayout: vi.fn(),
}))

let api: UseWorkspaceTabsReturn
let root: Root
let host: HTMLDivElement
let server: WorkspaceLayoutValue

/** A server layout with `count` single-pane tabs, the first one active. */
const strip = (count: number): WorkspaceLayoutValue => ({
  tabs: Array.from({ length: count }, (_, index) => ({
    id: `tab-${index + 1}`,
    columns: 1,
    screens: ['chat'],
  })),
  activeTabId: 'tab-1',
})

const client = () => ({
  browserId: 'browser-1',
  on: () => () => {},
  registerTrigger: () => () => {},
})

function Probe({ options }: { options?: UseWorkspaceTabsOptions }) {
  api = useWorkspaceTabs(options)
  return null
}

async function mount(options?: UseWorkspaceTabsOptions) {
  const qc = new QueryClient()
  await act(async () =>
    root.render(
      <QueryClientProvider client={qc}>
        <Probe options={options} />
      </QueryClientProvider>,
    ),
  )
}

/** Let promise chains and React Query's timer-scheduled notifications run. */
async function settle(ms = 20) {
  await act(async () => {
    await new Promise((done) => setTimeout(done, ms))
  })
}

beforeEach(() => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  vi.clearAllMocks()
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
  vi.restoreAllMocks()
  vi.unstubAllGlobals()
})

describe('workspace tab limit', () => {
  it('uses the default limit when the caller passes none', async () => {
    server = strip(1)
    await mount()
    await settle()
    expect(api.tabLimit).toBe(DEFAULT_TAB_LIMIT)
    expect(api.canCreateTab).toBe(true)
  })

  it('creates tabs up to the configured limit and refuses the next one', async () => {
    server = strip(1)
    await mount({ tabLimit: 2 })
    await settle()
    expect(api.canCreateTab).toBe(true)

    await act(async () => api.createTab({ columns: 1 }))
    await settle()
    expect(api.tabs).toHaveLength(2)
    expect(api.canCreateTab).toBe(false)
    const writes = vi.mocked(setWorkspaceLayout).mock.calls.length

    // At the limit: no new tab, no write, the active tab stays put.
    const activeBefore = api.activeTabId
    await act(async () => api.createTab({ columns: 1 }))
    await settle()
    expect(api.tabs).toHaveLength(2)
    expect(api.activeTabId).toBe(activeBefore)
    expect(vi.mocked(setWorkspaceLayout).mock.calls).toHaveLength(writes)
  })

  it('keeps tabs already open above a lowered limit, and lets them work', async () => {
    server = strip(4)
    await mount({ tabLimit: 2 })
    await settle()
    // Nothing is pruned, nothing is added.
    expect(api.tabs.map((tab) => tab.id)).toEqual([
      'tab-1',
      'tab-2',
      'tab-3',
      'tab-4',
    ])
    expect(api.canCreateTab).toBe(false)
    await act(async () => api.createTab({ columns: 1 }))
    await settle()
    expect(api.tabs).toHaveLength(4)

    // The existing tabs still switch, rename and close.
    await act(async () => api.activateTab('tab-3'))
    await settle()
    expect(api.activeTabId).toBe('tab-3')
    await act(async () => api.renameTab('tab-3', 'Review'))
    await settle()
    expect(api.tabs[2].name).toBe('Review')
    await act(async () => api.closeTab('tab-4'))
    await settle()
    expect(api.tabs).toHaveLength(3)
    // Still above the limit after one close: creation stays refused until
    // the count drops below it.
    expect(api.canCreateTab).toBe(false)
    await act(async () => api.closeTab('tab-3'))
    await act(async () => api.closeTab('tab-2'))
    await settle()
    expect(api.tabs).toHaveLength(1)
    expect(api.canCreateTab).toBe(true)
  })

  it('refuses inside the persisted transform when the server filled the strip first', async () => {
    server = strip(1)
    await mount({ tabLimit: 2 })
    await settle()
    expect(api.canCreateTab).toBe(true)
    // Another browser opened a tab between our last read and this write.
    server = strip(2)
    vi.mocked(fetchWorkspaceLayout).mockImplementation(async () =>
      structuredClone(server),
    )
    await act(async () => api.createTab({ columns: 1 }))
    await settle(50)
    // The write, if any, never carried a third tab.
    for (const [value] of vi.mocked(setWorkspaceLayout).mock.calls) {
      expect((value.tabs as unknown[]).length).toBeLessThanOrEqual(2)
    }
  })
})
