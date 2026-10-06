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

  it('does not count a subscription signed in on this machine', async () => {
    // A fresh project with only the Codex CLI signed in still opens the
    // wizard, so the person sees that subscription and chooses it.
    harness.providers = [provider('openai-codex', 3), provider('anthropic', 0)]
    harness.workers = new Set(['llm-router', 'provider-openai-codex'])
    await expect(connectedModelCount()).resolves.toBe(0)
    harness.providers.push(provider('openai', 4))
    harness.workers.add('provider-openai')
    await expect(connectedModelCount()).resolves.toBe(4)
  })

  it('does not count a provider that reports no credentials', async () => {
    // A local llama.cpp server fills the catalog before a key is connected;
    // the picker cannot use those models, so the wizard still opens.
    harness.providers = [{ ...provider('llamacpp', 2), configured: false }]
    harness.workers = new Set(['llm-router', 'provider-llamacpp'])
    await expect(connectedModelCount()).resolves.toBe(0)
  })

  it('does not count a provider whose worker left', async () => {
    harness.providers = [provider('anthropic', 11)]
    await expect(connectedModelCount()).resolves.toBe(0)
  })

  it('throws when the router cannot answer, so nothing opens on a guess', async () => {
    harness.routerDown = true
    await expect(connectedModelCount()).rejects.toThrow()
  })
})
