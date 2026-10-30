// @vitest-environment jsdom
import { act, useEffect } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { requestPanelOpen } from '@/lib/panel-context'
import {
  AgentProfilePicker,
  type AgentProfilePickerProps,
} from './AgentProfilePicker'
import { DEFAULT_SYSTEM_PROMPT_STATE } from './system-prompt-selection'

vi.mock('@/lib/panel-context', () => ({ requestPanelOpen: vi.fn() }))
vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
const entry = {
  id: 'default',
  name: 'Default',
  description: 'General tasks',
  logo: null,
  icon: null,
  model: null,
  skill_count: null,
  modified_at: '',
}
const reviewer = {
  ...entry,
  id: 'reviewer',
  name: 'Reviewer',
  model: 'test-model',
  reasoning_effort: 'high',
  composer_placeholder: 'Review this change.',
}
let root: Root | undefined
afterEach(async () => {
  await act(async () => root?.unmount())
  document.body.replaceChildren()
  vi.clearAllMocks()
})

async function mount(props: Partial<AgentProfilePickerProps> = {}) {
  const onAgentProfileChange = vi.fn()
  root = createRoot(document.body.appendChild(document.createElement('div')))
  await act(async () =>
    root?.render(
      <AgentProfilePicker
        systemPrompt={DEFAULT_SYSTEM_PROMPT_STATE}
        onSystemPromptChange={vi.fn()}
        onAgentProfileChange={onAgentProfileChange}
        agentEntries={[
          entry,
          reviewer,
          { ...entry, id: 'hidden', hidden: true },
        ]}
        {...props}
      />,
    ),
  )
  return onAgentProfileChange
}

async function open() {
  const trigger = document.querySelector<HTMLButtonElement>(
    '[aria-label^="Agent profile:"]',
  )
  await act(async () => trigger?.click())
}

describe('agent profile picker', () => {
  it('preserves a selected profile even when it is missing from the catalog', async () => {
    const change = await mount({
      agentProfile: { id: 'archived', name: 'My saved agent' },
      preselectDefaultAgent: true,
    })
    expect(
      document.querySelector('[aria-label="Agent profile: My saved agent"]'),
    ).not.toBeNull()
    expect(change).not.toHaveBeenCalled()
  })

  it.each([
    { systemPrompt: DEFAULT_SYSTEM_PROMPT_STATE },
    {
      systemPrompt: {
        ...DEFAULT_SYSTEM_PROMPT_STATE,
        choice: { named: 'custom-reviewer' },
      },
      preselectDefaultAgent: true,
    },
    {
      systemPrompt: DEFAULT_SYSTEM_PROMPT_STATE,
      preselectDefaultAgent: true,
      selectionLocked: true,
    },
  ])(
    'does not replace a non-draft, manual, or locked configuration',
    async (props) => {
      const change = await mount(props)
      expect(change).not.toHaveBeenCalled()
    },
  )

  it('selects the complete profile snapshot and excludes hidden profiles', async () => {
    const change = await mount()
    await open()
    expect(
      document.querySelector('[aria-label="Use hidden agent profile"]'),
    ).toBeNull()
    const choice = document.querySelector<HTMLButtonElement>(
      '[aria-label="Use Reviewer agent profile"]',
    )
    expect(choice).not.toBeNull()
    await act(async () => choice?.click())
    expect(change).toHaveBeenCalledWith(
      {
        id: 'reviewer',
        name: 'Reviewer',
        model: 'test-model',
        reasoningEffort: 'high',
        composerPlaceholder: 'Review this change.',
      },
      undefined,
    )
  })

  it('keeps the manual editor reachable with an empty catalog', async () => {
    await mount({ agentEntries: [] })
    await open()
    const configure = Array.from(document.querySelectorAll('button')).find(
      (button) => button.textContent?.includes('Configure an agent'),
    )
    expect(configure).toBeDefined()
    await act(async () => configure?.click())
    expect(requestPanelOpen).toHaveBeenCalledWith({
      pageId: 'directory',
      context: { collection: 'agents', action: 'create' },
    })
  })

  it('keeps worker controls mounted across profile-menu dismissals', async () => {
    const unmount = vi.fn()
    function WorkerControl() {
      useEffect(() => unmount, [])
      return <button type="button">Worker settings</button>
    }
    await mount({ details: <WorkerControl /> })
    await open()
    const choice = document.querySelector<HTMLButtonElement>(
      '[aria-label="Use Reviewer agent profile"]',
    )
    await act(async () => choice?.click())
    expect(unmount).not.toHaveBeenCalled()
    await open()
    expect(unmount).not.toHaveBeenCalled()
  })
})
