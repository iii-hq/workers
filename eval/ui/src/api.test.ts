import type { Host } from '@iii-dev/console-ui'
import { describe, expect, it, vi } from 'vitest'
import { createEvalApi } from './api'
import type { MonitorConfig } from './types'

function fakeApi() {
  const trigger = vi.fn().mockResolvedValue({})
  return { trigger, api: createEvalApi({ iii: { trigger } } as unknown as Host) }
}

const CONFIG: MonitorConfig = {
  enabled: true,
  model: { model: 'claude-sonnet-5', provider: 'anthropic' },
  code_repository: '/home/layon/workspaces/workers',
  revision: 'r1',
  updated_at: 1,
}

describe('eval::configure', () => {
  it('names the directory only when there is one: absent turns code access off', async () => {
    const { api, trigger } = fakeApi()
    await api.configure(true, CONFIG.model)
    await api.configure(true, CONFIG.model, '/srv/workers')
    expect(trigger.mock.calls[0][1]).toEqual({ enabled: true, model: CONFIG.model })
    expect(trigger.mock.calls[0][1]).not.toHaveProperty('code_repository')
    expect(trigger.mock.calls[1][1]).toEqual({ enabled: true, model: CONFIG.model, code_repository: '/srv/workers' })
  })

  it('keeps the saved directory when observation is paused and resumed', async () => {
    const { api, trigger } = fakeApi()
    await api.toggle(CONFIG)
    await api.toggle({ ...CONFIG, enabled: false })
    expect(trigger.mock.calls[0][0]).toBe('eval::configure')
    expect(trigger.mock.calls[0][1]).toEqual({
      enabled: false,
      model: CONFIG.model,
      code_repository: '/home/layon/workspaces/workers',
    })
    expect(trigger.mock.calls[1][1]).toMatchObject({
      enabled: true,
      code_repository: '/home/layon/workspaces/workers',
    })
    // A configuration without a directory stays without one.
    const { code_repository: _gone, ...plain } = CONFIG
    await api.toggle(plain)
    expect(trigger.mock.calls[2][1]).not.toHaveProperty('code_repository')
  })
})
