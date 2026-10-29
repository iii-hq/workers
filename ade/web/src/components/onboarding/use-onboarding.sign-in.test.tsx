// @vitest-environment jsdom

import { act } from 'react'
import { createRoot } from 'react-dom/client'
import { afterEach, describe, expect, it, vi } from 'vitest'
import type { PlanStep } from '@/lib/onboarding/plan'

;(
  globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }
).IS_REACT_ACT_ENVIRONMENT = true

const harness = vi.hoisted(() => {
  class SignInRequiredError extends Error {
    readonly providerId: string

    constructor(providerId: string, message: string) {
      super(message)
      this.providerId = providerId
    }
  }
  return { SignInRequiredError, outcomes: [] as Array<'rejected' | 'ok'> }
})

// Each wait-models step takes the next outcome: Codex turning the sign-in
// down, or its models arriving.
vi.mock('@/lib/onboarding/api', () => ({
  detectKeys: async () => [],
  installedWorkerNames: async () => new Set(['llm-router']),
  readableError: (error: unknown) =>
    error instanceof Error ? error.message : String(error),
  readConsoleConfig: async () => null,
  readProviderStates: async () => [],
  scanMachine: async () => [],
  SignInRequiredError: harness.SignInRequiredError,
  runStep: async (step: PlanStep) => {
    if (step.kind !== 'wait-models') return { note: 'done' }
    if (harness.outcomes.shift() === 'rejected') {
      throw new harness.SignInRequiredError(
        step.providerId,
        'Codex rejected the current session. Sign in with ChatGPT again.',
      )
    }
    return { note: '3 models' }
  },
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
  harness.outcomes = []
})

const WAIT_CODEX: PlanStep[] = [
  { kind: 'wait-models', providerId: 'openai-codex', title: 'Codex' },
]

async function mount() {
  const root = createRoot(document.createElement('div'))
  await act(async () => root.render(<Probe />))
  unmounts.push(() => act(() => root.unmount()))
  return () => latest as OnboardingController
}

describe('useOnboarding sign-in needed', () => {
  it('holds a rejected sign-in against the provider until its models arrive', async () => {
    harness.outcomes = ['rejected', 'ok']
    const hook = await mount()

    await act(async () => {
      expect(await hook().run('models', WAIT_CODEX)).toBe(false)
    })
    expect(hook().signInNeeded.has('openai-codex')).toBe(true)
    expect(hook().activity.at(-1)?.note).toBe(
      'Codex rejected the current session. Sign in with ChatGPT again.',
    )

    await act(async () => {
      expect(await hook().run('models', WAIT_CODEX)).toBe(true)
    })
    expect(hook().signInNeeded.has('openai-codex')).toBe(false)
  })

  it('lets a new scan try the local sign-in again', async () => {
    harness.outcomes = ['rejected']
    const hook = await mount()
    await act(async () => {
      await hook().run('models', WAIT_CODEX)
    })
    expect(hook().signInNeeded.has('openai-codex')).toBe(true)

    await act(async () => {
      await hook().refresh()
    })
    expect(hook().signInNeeded.size).toBe(0)
  })
})
