import { beforeEach, describe, expect, it, vi } from 'vitest'
import type { ProviderState } from '@/lib/onboarding/plan'

const harness = vi.hoisted(() => ({
  providers: [] as ProviderState[],
  workers: new Set<string>(),
  routerDown: false,
}))

vi.mock('@/lib/onboarding/api', () => ({
  readProviderStates: async () => {
    if (harness.routerDown) throw new Error('function_not_found')
    return harness.providers
  },
  installedWorkerNames: async () => harness.workers,
}))

import { connectedModelCount } from './use-onboarding'

const provider = (id: string, modelCount: number): ProviderState => ({
  id,
  title: id,
  configured: true,
  available: true,
  modelCount,
})

beforeEach(() => {
  harness.providers = []
  harness.workers = new Set(['llm-router'])
  harness.routerDown = false
})

describe('connectedModelCount', () => {
  it('reads zero on a project with nothing connected', async () => {
    harness.providers = [provider('anthropic', 0), provider('openai', 0)]
    await expect(connectedModelCount()).resolves.toBe(0)
  })

  it('counts the models a connected provider serves', async () => {
    harness.providers = [provider('anthropic', 9), provider('openai', 0)]
    harness.workers = new Set(['llm-router', 'provider-anthropic'])
    await expect(connectedModelCount()).resolves.toBe(9)
  })

  it('does not count a provider whose worker left', async () => {
    harness.providers = [provider('claude-code', 11)]
    await expect(connectedModelCount()).resolves.toBe(0)
  })

  it('throws when the router cannot answer, so nothing opens on a guess', async () => {
    harness.routerDown = true
    await expect(connectedModelCount()).rejects.toThrow()
  })
})
