// @vitest-environment jsdom

import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, describe, expect, it } from 'vitest'
import { TooltipProvider } from '@/components/ui/Tooltip'
import type { AgentEntry } from '@/lib/backend/directory-prompts'
import { getPanelContext, resetPanelContextForTests } from '@/lib/panel-context'
import { EmptyState } from './EmptyState'
import {
  DEFAULT_SYSTEM_PROMPT_STATE,
  withAgentChoice,
} from './system-prompt-selection'

;(
  globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }
).IS_REACT_ACT_ENVIRONMENT = true

const profile = (id: string, kit?: string, hidden = false): AgentEntry => ({
  id,
  name: id,
  description: `${id} profile.`,
  logo: null,
  icon: null,
  model: null,
  skill_count: null,
  hidden,
  source: kit ? { kind: 'kit', kit } : { kind: 'local' },
  modified_at: '2026-10-21T00:00:00.000Z',
})

let root: Root | null = null

afterEach(() => {
  act(() => root?.unmount())
  root = null
  document.body.innerHTML = ''
  resetPanelContextForTests()
})

function render(selected: string) {
  const container = document.body.appendChild(document.createElement('div'))
  root = createRoot(container)
  act(() =>
    root?.render(
      <TooltipProvider>
        <EmptyState
          variant="ready"
          systemPrompt={withAgentChoice(DEFAULT_SYSTEM_PROMPT_STATE, selected)}
          onSystemPromptChange={() => {}}
          agentEntries={[
            profile('lead', 'acme/team'),
            profile('reviewer', 'acme/team'),
            profile('worker', 'acme/team', true),
            profile('solo'),
          ]}
          kitUpdates={[
            {
              kit: 'acme/team',
              current: '1.1.0',
              available: '1.2.0',
              major: false,
            },
          ]}
        />
      </TooltipProvider>,
    ),
  )
  return container
}

const reviewAction = (view: HTMLElement) =>
  [...view.querySelectorAll('button')].filter((button) =>
    button.textContent?.startsWith('Review update'),
  )

describe('EmptyState kit updates', () => {
  it('offers the update only on the selected card and notes it on the kit’s other visible cards', () => {
    const view = render('lead')

    expect(reviewAction(view).map((button) => button.textContent)).toEqual([
      'Review update 1.2.0, acme/team kit',
    ])
    const reviewer = view.querySelector(
      '[aria-label="Use reviewer agent profile"]',
    )
    const note = document.getElementById(
      reviewer?.getAttribute('aria-describedby') ?? '',
    )
    expect(note?.textContent).toBe('Update 1.2.0 available')
    // `worker` is hidden and `solo` has no kit: one note, one action.
    expect(view.textContent?.match(/Update 1\.2\.0 available/g)).toHaveLength(1)
  })

  it('opens the kit in Directory → Kits', () => {
    const view = render('lead')

    act(() => reviewAction(view)[0]?.click())

    expect(getPanelContext('directory')?.context).toEqual({
      collection: 'kits',
      kit: 'acme/team',
    })
  })
})
