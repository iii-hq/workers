// @vitest-environment jsdom

import { act } from 'react'
import { createRoot } from 'react-dom/client'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type {
  ChromiumState,
  FollowInstallOptions,
} from '@/lib/onboarding/chromium'

;(
  globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }
).IS_REACT_ACT_ENVIRONMENT = true

const harness = vi.hoisted(() => ({
  workers: new Set<string>(),
  state: null as ChromiumState | null,
  reads: 0,
  follow: null as null | ((options: FollowInstallOptions) => Promise<unknown>),
}))

vi.mock('@/lib/onboarding/api', () => ({
  detectKeys: async () => [],
  installedWorkerNames: async () => harness.workers,
  readableError: (error: unknown) => String(error),
  readConsoleConfig: async () => null,
  readProviderStates: async () => [],
  runStep: async () => ({}),
  scanMachine: async () => [],
}))

vi.mock('@/lib/secrets', async (importOriginal) => ({
  ...(await importOriginal<typeof import('@/lib/secrets')>()),
  getSecretsStatus: async () => undefined,
}))

vi.mock('@/lib/onboarding/chromium', async (importOriginal) => ({
  ...(await importOriginal<typeof import('@/lib/onboarding/chromium')>()),
  readChromiumState: async () => {
    harness.reads += 1
    return harness.state
  },
  followChromiumInstall: (options: FollowInstallOptions) =>
    harness.follow?.(options),
}))

import { type OnboardingController, useOnboarding } from './use-onboarding'

const MISSING: ChromiumState = {
  found: false,
  engine: 'chromium',
  canInstall: true,
  approxDownloadMb: 170,
  legacy: false,
}

let latest: OnboardingController | null = null
const unmounts: Array<() => void> = []

function Probe() {
  latest = useOnboarding(true)
  return null
}

async function mount() {
  const container = document.createElement('div')
  const root = createRoot(container)
  await act(async () => root.render(<Probe />))
  unmounts.push(() => act(() => root.unmount()))
  if (!latest) throw new Error('not mounted')
  return () => latest as OnboardingController
}

beforeEach(() => {
  harness.workers = new Set(['llm-router'])
  harness.state = MISSING
  harness.reads = 0
  harness.follow = null
  latest = null
})

afterEach(() => {
  for (const unmount of unmounts.splice(0)) unmount()
})

describe('useOnboarding: Chromium', () => {
  it('does not look for Chromium without the browser worker', async () => {
    const current = await mount()
    expect(harness.reads).toBe(0)
    expect(current().snapshot.browser).toBeNull()
  })

  it('reads Chromium when the browser worker is installed', async () => {
    harness.workers = new Set(['llm-router', 'browser'])
    const current = await mount()
    expect(harness.reads).toBe(1)
    expect(current().snapshot.browser).toEqual(MISSING)
  })

  it('logs the download as a setup action and keeps each progress event', async () => {
    harness.workers = new Set(['browser'])
    harness.follow = async ({ onProgress }) => {
      onProgress({
        job_id: 'job-1',
        phase: 'downloading',
        bytes_done: 50,
        bytes_total: 100,
        timestamp: 1,
      })
      harness.state = { ...MISSING, found: true, version: '131.0' }
      return { ok: true, version: '131.0', path: '/c/chrome' }
    }
    const current = await mount()
    let outcome: unknown
    await act(async () => {
      outcome = await current().installChromium()
    })
    expect(outcome).toEqual({ ok: true, version: '131.0', path: '/c/chrome' })
    const entry = current().activity.find((item) => item.group === 'browser')
    expect(entry).toMatchObject({
      status: 'done',
      detail: 'browser::chromium::install',
      note: 'Chromium 131.0 · /c/chrome',
    })
    expect(current().chromiumProgress?.phase).toBe('downloading')
    expect(current().snapshot.browser?.found).toBe(true)
    expect(current().running).toBeNull()
  })

  it('logs a failed download with its hint', async () => {
    harness.workers = new Set(['browser'])
    harness.follow = async () => ({
      ok: false,
      error: 'Chromium does not start.',
      hint: 'Install libnss3.',
    })
    const current = await mount()
    await act(async () => {
      await current().installChromium()
    })
    expect(
      current().activity.find((item) => item.group === 'browser'),
    ).toMatchObject({
      status: 'failed',
      note: 'Chromium does not start. Install libnss3.',
    })
  })
})
