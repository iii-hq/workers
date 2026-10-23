import { beforeEach, describe, expect, it, vi } from 'vitest'
import { shouldAutoOpenOnboarding } from '@/lib/onboarding/open'
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

  it('counts a provider that owns its authentication', () => {
    // Codex or Copilot signed in: the router holds no credential, so it
    // reports configured: false, but the models are usable. First run opens
    // the wizard on its status alone, so these count as connected.
    harness.providers = [
      {
        ...provider('openai-codex', 3),
        configured: false,
        ownsAuthentication: true,
      },
      provider('anthropic', 0),
    ]
    harness.workers = new Set(['llm-router', 'provider-openai-codex'])
    return expect(connectedModelCount()).resolves.toBe(3)
  })

  it('keeps a finished project that uses only Codex closed', async () => {
    // Setup finished with Codex alone: no key provider is configured, and
    // the wizard must not open again on every visit.
    harness.providers = [
      {
        ...provider('openai-codex', 3),
        configured: false,
        ownsAuthentication: true,
      },
      { ...provider('anthropic', 0), configured: false },
      { ...provider('openai', 0), configured: false },
    ]
    harness.workers = new Set(['llm-router', 'provider-openai-codex'])
    const models = await connectedModelCount()
    expect(models).toBe(3)
    for (const status of ['completed', 'dismissed']) {
      expect(shouldAutoOpenOnboarding({ status }, false, models)).toBe(false)
    }
    // Signed out of Codex later: nothing is connected, so it opens again.
    harness.providers[0] = { ...harness.providers[0], modelCount: 0 }
    expect(
      shouldAutoOpenOnboarding(
        { status: 'completed' },
        false,
        await connectedModelCount(),
      ),
    ).toBe(true)
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
