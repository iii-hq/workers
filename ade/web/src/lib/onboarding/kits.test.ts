import { beforeEach, describe, expect, it, vi } from 'vitest'

const harness = vi.hoisted(() => ({
  kits: [] as unknown,
  answers: new Map<string, unknown>(),
  calls: [] as { fn: string; payload: unknown; timeoutMs?: number }[],
}))

const MISSING = { code: 'function_not_found' }

vi.mock('@/lib/iii-client', () => ({
  getIiiClient: async () => ({
    trigger: async (
      fn: string,
      payload: { kit?: string },
      options?: { timeoutMs?: number },
    ) => {
      harness.calls.push({ fn, payload, timeoutMs: options?.timeoutMs })
      if (fn === 'console::onboarding::prompts') return { kits: harness.kits }
      const answer = harness.answers.get(payload.kit ?? '')
      if (answer instanceof Error || answer === MISSING) throw answer
      return answer
    },
  }),
}))
vi.mock('@/lib/secrets', () => ({
  isMissingFunction: (error: unknown) => error === MISSING,
}))
vi.mock('./api', () => ({
  readableError: (error: unknown) =>
    error instanceof Error ? error.message : String(error),
}))
vi.mock('./prompts', () => ({
  PROMPTS_FUNCTION: 'console::onboarding::prompts',
}))

import {
  installKit,
  KIT_INSTALL_TIMEOUT_MS,
  onboardingKitsSnapshot,
  parseOnboardingKits,
  resetOnboardingKitsForTest,
  retryOnboardingKit,
  startOnboardingKits,
  subscribeOnboardingKits,
} from './kits'

beforeEach(() => {
  resetOnboardingKitsForTest()
  harness.kits = []
  harness.answers.clear()
  harness.calls = []
})

const installs = () =>
  harness.calls.filter((call) => call.fn === 'directory::download-kit')

describe('parseOnboardingKits', () => {
  it('keeps trimmed, distinct strings', () => {
    expect(
      parseOnboardingKits({
        kits: [' iii/deepwork ', 'iii/deepwork', 7, '', 'iii/x'],
      }),
    ).toEqual(['iii/deepwork', 'iii/x'])
    expect(parseOnboardingKits({ prompts: [] })).toEqual([])
    expect(parseOnboardingKits(null)).toEqual([])
  })
})

describe('installKit', () => {
  it('installs with apply and a long timeout, and maps the Directory status', async () => {
    harness.answers.set('a/applied', { status: 'applied' })
    harness.answers.set('a/same', { status: 'up_to_date' })
    harness.answers.set('a/review', {
      status: 'planned',
      message: 'Not applied automatically: the plan needs review.',
      plan: { plan_id: 'kp_1' },
    })
    harness.answers.set('a/odd', { status: 'mystery' })
    harness.answers.set('a/down', new Error('registry unreachable'))
    expect(await installKit('a/applied')).toEqual({
      kit: 'a/applied',
      state: 'installed',
    })
    expect(await installKit('a/same')).toEqual({
      kit: 'a/same',
      state: 'installed',
    })
    expect(await installKit('a/review')).toEqual({
      kit: 'a/review',
      state: 'review',
      message: 'Not applied automatically: the plan needs review.',
      planId: 'kp_1',
    })
    expect((await installKit('a/odd'))?.state).toBe('failed')
    expect(await installKit('a/down')).toEqual({
      kit: 'a/down',
      state: 'failed',
      message: 'registry unreachable',
    })
    expect(installs()[0]).toEqual({
      fn: 'directory::download-kit',
      payload: { kit: 'a/applied', apply: true },
      timeoutMs: KIT_INSTALL_TIMEOUT_MS,
    })
  })

  it('gives null when the Directory has no kit support', async () => {
    harness.answers.set('a/kit', MISSING)
    expect(await installKit('a/kit')).toBeNull()
  })
})

describe('startOnboardingKits', () => {
  it('installs the template kits one at a time, once per page load', async () => {
    harness.kits = ['iii/deepwork', 'iii/other']
    harness.answers.set('iii/deepwork', { status: 'applied' })
    harness.answers.set('iii/other', { status: 'planned' })
    const seen: string[] = []
    subscribeOnboardingKits(() =>
      seen.push(
        onboardingKitsSnapshot()
          .map((entry) => `${entry.kit}:${entry.state}`)
          .join(','),
      ),
    )
    const first = startOnboardingKits()
    const second = startOnboardingKits()
    await Promise.all([first, second])
    expect(
      installs().map((call) => (call.payload as { kit: string }).kit),
    ).toEqual(['iii/deepwork', 'iii/other'])
    expect(seen[0]).toBe('iii/deepwork:installing')
    expect(onboardingKitsSnapshot().map((entry) => entry.state)).toEqual([
      'installed',
      'review',
    ])
    await startOnboardingKits()
    expect(installs()).toHaveLength(2)
  })

  it('shows nothing for a Directory without kits, and retries one kit', async () => {
    harness.kits = ['iii/gone', 'iii/flaky']
    harness.answers.set('iii/gone', MISSING)
    harness.answers.set('iii/flaky', new Error('timed out'))
    await startOnboardingKits()
    expect(onboardingKitsSnapshot()).toEqual([
      { kit: 'iii/flaky', state: 'failed', message: 'timed out' },
    ])
    harness.answers.set('iii/flaky', { status: 'up_to_date' })
    await retryOnboardingKit('iii/flaky')
    expect(onboardingKitsSnapshot()).toEqual([
      { kit: 'iii/flaky', state: 'installed' },
    ])
  })
})
