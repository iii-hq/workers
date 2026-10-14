// @vitest-environment jsdom

import { act } from 'react'
import { createRoot } from 'react-dom/client'
import { afterEach, describe, expect, it, vi } from 'vitest'
import type { PlanStep, ProviderState } from '@/lib/onboarding/plan'

;(
  globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }
).IS_REACT_ACT_ENVIRONMENT = true

const CLAUDE: ProviderState = {
  id: 'claude-code',
  title: 'Claude Code',
  configured: false,
  available: true,
  modelCount: 12,
  ownsAuthentication: true,
}
const COPILOT: ProviderState = {
  id: 'github-copilot',
  title: 'GitHub Copilot',
  configured: false,
  available: true,
  modelCount: 4,
  ownsAuthentication: true,
}

const harness = vi.hoisted(() => ({
  /** What each successive `readProviderStates` answers; the last repeats. */
  answers: [] as ProviderState[][],
  reads: 0,
}))

vi.mock('@/lib/onboarding/api', () => ({
  detectKeys: async () => [],
  installedWorkerNames: async () =>
    new Set(['llm-router', 'provider-claude-code', 'provider-github-copilot']),
  readableError: (error: unknown) => String(error),
  readConsoleConfig: async () => null,
  readProviderStates: async () => {
    const answer =
      harness.answers[Math.min(harness.reads, harness.answers.length - 1)]
    harness.reads += 1
    return answer
  },
  scanMachine: async () => [],
  runStep: async () => ({ note: '4 models' }),
}))

vi.mock('@/lib/secrets', async (importOriginal) => ({
  ...(await importOriginal<typeof import('@/lib/secrets')>()),
  getSecretsStatus: async () => undefined,
}))

import { type OnboardingController, useOnboarding } from './use-onboarding'

let latest: OnboardingController | null = null
const unmounts: Array<() => void> = []

function Probe() {
  latest = useOnboarding(true)
  return null
}

afterEach(() => {
  for (const unmount of unmounts.splice(0)) unmount()
  harness.answers = []
  harness.reads = 0
})

async function mount() {
  const root = createRoot(document.createElement('div'))
  await act(async () => root.render(<Probe />))
  unmounts.push(() => act(() => root.unmount()))
  return () => latest as OnboardingController
}

const SIGNED_IN: PlanStep[] = [
  {
    kind: 'wait-models',
    providerId: 'github-copilot',
    title: 'GitHub Copilot',
  },
]

const serving = (hook: () => OnboardingController) =>
  (hook().snapshot.providers ?? [])
    .filter((provider) => provider.modelCount > 0)
    .map((provider) => provider.id)

describe('useOnboarding after a sign-in', () => {
  it('looks again when a provider it did not touch blinks out', async () => {
    // Mount: Claude Code serves. After Copilot signs in, the first look
    // catches Claude Code re-registering; the next finds it back.
    harness.answers = [
      [CLAUDE],
      [{ ...CLAUDE, available: false, modelCount: 0 }, COPILOT],
      [CLAUDE, COPILOT],
    ]
    const hook = await mount()
    expect(serving(hook)).toEqual(['claude-code'])

    await act(async () => {
      await hook().run('models', SIGNED_IN)
    })
    expect(serving(hook).sort()).toEqual(['claude-code', 'github-copilot'])
    expect(harness.reads).toBe(3)
    expect(hook().running).toBeNull()
  }, 10_000)

  it('does not wait on a provider the run removed', async () => {
    harness.answers = [[CLAUDE], []]
    const hook = await mount()
    await act(async () => {
      await hook().run('models', [
        { kind: 'remove-workers', workers: ['provider-claude-code'] },
      ])
    })
    // One look after the run, no more: Claude Code was meant to go.
    expect(harness.reads).toBe(2)
    expect(serving(hook)).toEqual([])
  })
})
