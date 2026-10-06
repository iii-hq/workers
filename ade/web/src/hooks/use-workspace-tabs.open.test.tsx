// @vitest-environment jsdom
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import {
  fetchWorkspaceLayout,
  setWorkspaceLayout,
  type WorkspaceLayoutValue,
} from '@/lib/workspace-layout'
import {
  type UseWorkspaceTabsReturn,
  useWorkspaceTabs,
} from './use-workspace-tabs'

vi.mock('@/lib/workspace-layout', async (original) => ({
  ...(await original<typeof import('@/lib/workspace-layout')>()),
  fetchWorkspaceLayout: vi.fn(),
  setWorkspaceLayout: vi.fn(),
}))

let api: UseWorkspaceTabsReturn
let root: Root
let host: HTMLDivElement
let server: WorkspaceLayoutValue

function Probe() {
  api = useWorkspaceTabs()
  return null
}

async function settle() {
  await act(async () => {
    await new Promise((done) => setTimeout(done, 20))
  })
}

beforeEach(async () => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  vi.clearAllMocks()
  server = {
    tabs: [{ id: 'tab-home', columns: 2, screens: ['chat', 'ext:onboarding'] }],
    activeTabId: 'tab-home',
  }
  vi.mocked(fetchWorkspaceLayout).mockImplementation(async () =>
    structuredClone(server),
  )
  vi.mocked(setWorkspaceLayout).mockImplementation(async (value) => {
    server = structuredClone(value)
  })
  host = document.createElement('div')
  document.body.append(host)
  root = createRoot(host)
  await act(async () =>
    root.render(
      <QueryClientProvider client={new QueryClient()}>
        <Probe />
      </QueryClientProvider>,
    ),
  )
  await settle()
})

afterEach(async () => {
  await act(async () => root.unmount())
  host.remove()
  vi.restoreAllMocks()
  vi.unstubAllGlobals()
})

describe('openScreen with a placement', () => {
  it('lands beside the named anchor with the requested widths', async () => {
    await act(async () =>
      api.openScreen('traces', {
        relativeTo: 'ext:onboarding',
        direction: 'right',
        sizes: [3, 4, 3],
      }),
    )
    // React Query notifies observers on a timer tick, not inside act().
    await settle()
    expect(api.tabs[0].screens).toEqual(['chat', 'ext:onboarding', 'traces'])
    expect(api.tabs[0].sizes).toEqual([0.3, 0.4, 0.3])
    expect(server.tabs).toEqual([
      expect.objectContaining({
        screens: ['chat', 'ext:onboarding', 'traces'],
        sizes: [0.3, 0.4, 0.3],
      }),
    ])
  })

  it('keeps the widths of a tab whose screen it only reuses', async () => {
    await act(async () =>
      api.openScreen('traces', {
        relativeTo: 'ext:onboarding',
        sizes: [3, 4, 3],
      }),
    )
    await settle()
    await act(async () => api.openScreen('traces', { sizes: [1, 1, 1] }))
    await settle()
    expect(api.tabs[0].sizes).toEqual([0.3, 0.4, 0.3])
  })
})
