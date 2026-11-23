// @vitest-environment jsdom

import { act, useSyncExternalStore } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { requestOnboardingWizard } from '@/lib/onboarding/open'
import type { ActivityEntry, MachineSnapshot } from './use-onboarding'

;(
  globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }
).IS_REACT_ACT_ENVIRONMENT = true

/*
 * The wizard saves its completion summary when a setup step moves on. The
 * step calls `onNext` from a callback made BEFORE its run (Connect, a
 * Chromium download); these tests call such a stale callback after the run
 * has changed the state, and check the summary reflects the settled state.
 */

const harness = vi.hoisted(() => ({
  saved: [] as { status: string; summary: unknown }[],
  modelsNext: [] as (() => void)[],
  browserNext: [] as (() => void)[],
  listeners: new Set<() => void>(),
  controller: null as unknown,
}))

vi.mock('@/lib/onboarding/api', () => ({
  fetchOnboardingState: async () => null,
  readableError: (error: unknown) => String(error),
  saveOnboardingState: async (status: string, summary: unknown) => {
    harness.saved.push({ status, summary })
  },
}))
vi.mock('@/lib/telemetry', () => ({ setupCompleted: () => undefined }))
vi.mock('@/lib/onboarding/telemetry', () => ({
  finishStep: () => ({}),
  reportSetupStep: () => undefined,
}))
vi.mock('./ModelsStep', () => ({
  ModelsStep: ({ onNext }: { onNext: () => void }) => {
    harness.modelsNext.push(onNext)
    return <footer data-step="models" />
  },
}))
vi.mock('./BrowserStep', () => ({
  BrowserStep: ({ onNext }: { onNext: () => void }) => {
    harness.browserNext.push(onNext)
    return <footer data-step="browser" />
  },
}))
vi.mock('./use-onboarding', () => {
  const subscribe = (listener: () => void) => {
    harness.listeners.add(listener)
    return () => harness.listeners.delete(listener)
  }
  return {
    connectedModelCount: async () => 0,
    useOnboarding: () =>
      useSyncExternalStore(subscribe, () => harness.controller),
  }
})

const { OnboardingWizardHost } = await import('./OnboardingWizard')

function controller(
  snapshot: Partial<MachineSnapshot>,
  activity: ActivityEntry[] = [],
) {
  return {
    snapshot: {
      tools: [],
      toolsError: null,
      providers: [],
      providersError: null,
      detections: null,
      envFile: '.env',
      installed: new Set<string>(),
      consoleConfig: null,
      browser: null,
      browserError: null,
      ...snapshot,
    },
    scanning: false,
    refresh: async () => undefined,
    activity,
    running: null,
    run: async () => true,
    signInNeeded: new Set<string>(),
    skipStep: () => undefined,
    checkChromium: async () => null,
    installChromium: async () => ({ ok: true as const }),
    chromiumProgress: null,
  }
}

function settle(next: unknown) {
  harness.controller = next
  for (const listener of harness.listeners) listener()
}

let container: HTMLDivElement
let root: Root

beforeEach(() => {
  harness.saved = []
  harness.modelsNext = []
  harness.browserNext = []
  harness.controller = controller({})
  container = document.createElement('div')
  document.body.append(container)
  root = createRoot(container)
  act(() => root.render(<OnboardingWizardHost />))
})

afterEach(() => {
  act(() => root.unmount())
  container.remove()
})

const completed = () =>
  harness.saved.filter((entry) => entry.status === 'completed')

describe('setup summary after a run', () => {
  it('saves the providers and workers a Connect added, not the state before it', () => {
    act(() => requestOnboardingWizard('models'))
    const stale = harness.modelsNext.at(-1)
    if (!stale) throw new Error('Models did not render')
    act(() =>
      settle(
        controller(
          {
            providers: [
              {
                id: 'claude-code',
                title: 'Claude Code',
                configured: false,
                available: true,
                ownsAuthentication: true,
                modelCount: 13,
              },
            ],
          },
          [
            {
              id: 1,
              group: 'models',
              title: 'Add the provider-claude-code worker',
              status: 'done',
              workers: ['provider-claude-code'],
            } as ActivityEntry,
          ],
        ),
      ),
    )
    act(() => stale())
    expect(completed()).toHaveLength(1)
    expect(completed()[0].summary).toMatchObject({
      providers: [{ id: 'claude-code', models: 13 }],
      workers_added: ['provider-claude-code'],
    })
  })

  it('saves Chromium as installed after a download that finished', () => {
    act(() => requestOnboardingWizard('browser'))
    const stale = harness.browserNext.at(-1)
    if (!stale) throw new Error('Browser did not render')
    act(() =>
      settle(
        controller({ browser: { found: true, version: '131.0' } as never }, [
          {
            id: 2,
            group: 'browser',
            title: 'Download Chromium',
            status: 'done',
          } as ActivityEntry,
        ]),
      ),
    )
    act(() => stale())
    expect(completed()).toHaveLength(1)
    expect(completed()[0].summary).toMatchObject({
      chromium_installed: '131.0',
    })
  })

  it('does not complete setup from a run that outlived a closed visit', () => {
    act(() => requestOnboardingWizard('browser'))
    const stale = harness.browserNext.at(-1)
    if (!stale) throw new Error('Browser did not render')
    const close = document.querySelector<HTMLButtonElement>(
      'button[aria-label="Close"]',
    )
    if (!close) throw new Error('no close button')
    act(() => close.click())
    act(() => requestOnboardingWizard('browser'))
    act(() => stale())
    expect(completed()).toHaveLength(0)
  })
})
