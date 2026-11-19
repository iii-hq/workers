// @vitest-environment jsdom

import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { AgentEntry } from '@/lib/backend/directory-prompts'
import { resetOnboardingKitsForTest } from '@/lib/onboarding/kits'
import { AgentProfilePicker } from './AgentProfilePicker'
import { DEFAULT_SYSTEM_PROMPT_STATE } from './system-prompt-selection'

const harness = vi.hoisted(() => ({
  agents: [] as AgentEntry[],
  failures: 0,
  handlers: new Map<string, (payload: unknown) => void>(),
  observed: [] as Array<{ source: unknown; target: string }>,
  listCalls: 0,
  downloads: 0,
  kitProfile: null as unknown as AgentEntry,
}))

vi.mock('@/lib/panel-context', () => ({ requestPanelOpen: vi.fn() }))
vi.mock('@/lib/iii-client', () => ({
  getIiiClient: async () => ({
    browserId: 'browser-1',
    call: async (functionId: string) => {
      if (functionId === 'directory::agents::list') {
        harness.listCalls += 1
        if (harness.failures > 0) {
          harness.failures -= 1
          throw new Error('directory unavailable')
        }
        return { agents: harness.agents }
      }
      if (functionId === 'directory::kits::list') return { kits: [] }
      if (functionId === 'directory::download-kit') {
        harness.downloads += 1
        harness.agents = [...harness.agents, harness.kitProfile]
        for (const handler of harness.handlers.values()) handler({ op: 'kit' })
        return { status: 'applied' }
      }
      throw new Error(`unexpected call ${functionId}`)
    },
    on: (functionId: string, handler: (payload: unknown) => void) => {
      harness.handlers.set(functionId, handler)
      return () => harness.handlers.delete(functionId)
    },
    observe: (source: unknown, target: string) => {
      harness.observed.push({ source, target })
      return () => {}
    },
  }),
}))
vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)

const profile = (id: string, name: string): AgentEntry => ({
  id,
  name,
  description: `${name} profile.`,
  logo: null,
  icon: null,
  model: null,
  skill_count: null,
  modified_at: '',
})

let root: Root | undefined

beforeEach(() => {
  harness.agents = [profile('default', 'Default')]
  harness.failures = 0
  harness.handlers.clear()
  harness.observed = []
  harness.listCalls = 0
  harness.downloads = 0
  harness.kitProfile = profile('deepwork', 'Deepwork')
  resetOnboardingKitsForTest()
})

afterEach(async () => {
  await act(async () => root?.unmount())
  root = undefined
  document.body.replaceChildren()
})

const flush = () => act(async () => {})

async function mount() {
  const onAgentProfileChange = vi.fn()
  root = createRoot(document.body.appendChild(document.createElement('div')))
  await act(async () =>
    root?.render(
      <AgentProfilePicker
        systemPrompt={DEFAULT_SYSTEM_PROMPT_STATE}
        onSystemPromptChange={vi.fn()}
        onAgentProfileChange={onAgentProfileChange}
      />,
    ),
  )
  await flush()
  return onAgentProfileChange
}

const kitRow = () =>
  document.querySelector<HTMLButtonElement>(
    '[aria-label="Install the Deepwork agent"], [aria-label="Deepwork agent is installing"]',
  )

const trigger = () =>
  document.querySelector<HTMLButtonElement>('.composer-agent-trigger')

const profileNames = () =>
  [...document.querySelectorAll('[aria-label$=" agent profile"]')].map(
    (button) => button.getAttribute('aria-label'),
  )

/** Ring the Directory's agents doorbell, as a kit apply that wrote profiles does. */
async function agentsChanged() {
  const observer = harness.observed.find(
    ({ source }) =>
      (source as { function_id?: string }).function_id ===
      'directory::agents::on-change',
  )
  expect(observer).toBeDefined()
  const handler = harness.handlers.get(
    observer?.target.replace(/::browser-1$/, '') ?? '',
  )
  expect(handler).toBeDefined()
  await act(async () => handler?.({ op: 'kit', kit: 'iii/deepwork' }))
  await flush()
}

describe('AgentProfilePicker catalog', () => {
  it('shows a kit’s profile once it lands, without a reload', async () => {
    await mount()
    await act(async () => trigger()?.click())
    expect(profileNames()).toEqual(['Use Default agent profile'])

    // The setup wizard's background install finishes after the chat mounted.
    harness.agents = [
      profile('default', 'Default'),
      profile('deepwork', 'Deepwork'),
    ]
    await agentsChanged()

    expect(profileNames()).toEqual([
      'Use Default agent profile',
      'Use Deepwork agent profile',
    ])
  })

  it('reads the profiles again each time the picker opens', async () => {
    await mount()
    const before = harness.listCalls
    harness.agents = [
      profile('default', 'Default'),
      profile('deepwork', 'Deepwork'),
    ]

    await act(async () => trigger()?.click())
    await flush()

    expect(harness.listCalls).toBe(before + 1)
    expect(profileNames()).toContain('Use Deepwork agent profile')
  })

  it('keeps the list it has when a later read fails', async () => {
    await mount()
    harness.failures = 1
    await agentsChanged()
    await act(async () => trigger()?.click())
    await flush()

    // The open's read succeeds; the failed doorbell read never blanked it.
    expect(profileNames()).toEqual(['Use Default agent profile'])
    expect(document.body.textContent).not.toContain('Unable to load agents.')
  })

  it('offers Retry when the first read fails, and recovers', async () => {
    // The read on mount and the read on open both fail.
    harness.failures = 2
    await mount()
    await act(async () => trigger()?.click())
    expect(document.body.textContent).toContain('Unable to load agents.')

    const retry = [...document.querySelectorAll('button')].find(
      (button) => button.textContent === 'Retry',
    )
    await act(async () => retry?.click())
    await flush()

    expect(document.body.textContent).not.toContain('Unable to load agents.')
    expect(profileNames()).toEqual(['Use Default agent profile'])
  })

  it('always offers Deepwork, and installs and selects it on click', async () => {
    const onAgentProfileChange = await mount()
    await act(async () => trigger()?.click())
    expect(kitRow()?.textContent).toContain('Install')

    await act(async () => kitRow()?.click())
    await flush()
    await flush()

    expect(harness.downloads).toBe(1)
    expect(kitRow()).toBeNull()
    expect(profileNames()).toContain('Use Deepwork agent profile')
    expect(onAgentProfileChange).toHaveBeenLastCalledWith(
      expect.objectContaining({ id: 'deepwork' }),
      undefined,
    )
  })

  it('hides the install row once the Directory serves Deepwork', async () => {
    harness.agents = [
      profile('default', 'Default'),
      profile('deepwork', 'Deepwork'),
    ]
    await mount()
    await act(async () => trigger()?.click())
    expect(kitRow()).toBeNull()
    expect(profileNames()).toEqual([
      'Use Default agent profile',
      'Use Deepwork agent profile',
    ])
  })
})
