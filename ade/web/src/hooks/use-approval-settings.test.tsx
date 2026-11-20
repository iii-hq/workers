// @vitest-environment jsdom

import { act } from 'react'
import { createRoot } from 'react-dom/client'
import { describe, expect, it, vi } from 'vitest'
import type { ApprovalSettings } from '@/lib/backend/approval-settings'
import { useApprovalSettings } from './use-approval-settings'

;(
  globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }
).IS_REACT_ACT_ENVIRONMENT = true

const base: ApprovalSettings = {
  mode: 'manual',
  always_allow: [],
  approved_always: [],
  mode_set_at: 1,
}

vi.mock('@/lib/backend/approval-gate-config', () => ({
  loadApprovalGateDefaults: vi.fn(),
}))
vi.mock('@/lib/backend/approval-settings', async (orig) => ({
  ...(await orig<typeof import('@/lib/backend/approval-settings')>()),
  getApprovalSettings: vi.fn(async () => base),
  // Never settles: the test observes the optimistic state.
  approveAlways: vi.fn(() => new Promise(() => {})),
}))

async function mount() {
  let hook: ReturnType<typeof useApprovalSettings> | undefined
  function Probe() {
    hook = useApprovalSettings('s')
    return null
  }
  const root = createRoot(document.createElement('div'))
  await act(async () => root.render(<Probe />))
  return () => hook as ReturnType<typeof useApprovalSettings>
}

describe('approveAlways optimistic entry', () => {
  it('carries the namespace and dedups on (namespace, function_id)', async () => {
    const get = await mount()
    await act(async () => {
      void get().approveAlways('state::list', 'laptop')
      void get().approveAlways('state::list', 'other')
      void get().approveAlways('state::list', 'laptop')
      void get().approveAlways('state::list')
      void get().approveAlways('state::list')
    })
    expect(
      get().settings.approved_always.map((e) => [e.namespace, e.function_id]),
    ).toEqual([
      ['laptop', 'state::list'],
      ['other', 'state::list'],
      [undefined, 'state::list'],
    ])
    expect(get().settings.approved_always[2]).not.toHaveProperty('namespace')
  })
})
