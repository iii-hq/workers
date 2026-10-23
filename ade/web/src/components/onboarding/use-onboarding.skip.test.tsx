// @vitest-environment jsdom

import { act } from 'react'
import { createRoot } from 'react-dom/client'
import { afterEach, describe, expect, it, vi } from 'vitest'
import type { RunContext } from '@/lib/onboarding/api'
import type { PlanStep } from '@/lib/onboarding/plan'

;(
  globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }
).IS_REACT_ACT_ENVIRONMENT = true

const harness = vi.hoisted(() => ({
  signals: [] as RunContext['signal'][],
  ran: [] as string[],
}))

// The first step hangs (a provider that never answers); every other one
// finishes at once.
vi.mock('@/lib/onboarding/api', () => ({
  detectKeys: async () => [],
  installedWorkerNames: async () => new Set(['llm-router']),
  readableError: (error: unknown) => String(error),
  readConsoleConfig: async () => null,
  readProviderStates: async () => [],
  scanMachine: async () => [],
  runStep: (step: PlanStep, context: RunContext) => {
    harness.ran.push(step.kind)
    harness.signals.push(context.signal)
    context.report({ note: `Working on ${step.kind}` })
    if (harness.ran.length === 1) return new Promise(() => undefined)
    return Promise.resolve({ note: 'done' })
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
  harness.signals = []
  harness.ran = []
})

const PLAN: PlanStep[] = [
  { kind: 'wait-models', providerId: 'openai', title: 'OpenAI' },
  { kind: 'wait-models', providerId: 'anthropic', title: 'Anthropic' },
]

describe('useOnboarding skip', () => {
  it('skips the slow step, runs the next one, and the plan still succeeds', async () => {
    const root = createRoot(document.createElement('div'))
    await act(async () => root.render(<Probe />))
    unmounts.push(() => act(() => root.unmount()))
    const hook = () => latest as OnboardingController

    let result: Promise<boolean> | undefined
    await act(async () => {
      result = hook().run('models', PLAN)
    })
    const first = hook().activity[0]
    expect(first?.status).toBe('running')
    expect(first?.startedAt).toBeTypeOf('number')
    expect(first?.note).toBe('Working on wait-models')
    expect(first?.noteAt).toBeTypeOf('number')

    await act(async () => {
      hook().skipStep()
      await result
    })
    expect(await result).toBe(true)
    // The skipped step's own waits were told to stop.
    expect(harness.signals[0]?.cancelled).toBe(true)
    expect(harness.ran).toHaveLength(2)
    expect(hook().activity.map((entry) => entry.status)).toEqual([
      'skipped',
      'done',
    ])
    expect(hook().running).toBeNull()
  })
})
