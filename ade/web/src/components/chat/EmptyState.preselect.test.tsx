// @vitest-environment jsdom

import { act, useLayoutEffect } from 'react'
import { createRoot } from 'react-dom/client'
import { afterEach, describe, expect, it } from 'vitest'
import type { AgentEntry } from '@/lib/backend/directory-prompts'
import type {
  AgentProfileChangeOptions,
  AgentProfileSnapshot,
} from '@/types/chat'
import { EmptyState } from './EmptyState'
import { DEFAULT_SYSTEM_PROMPT_STATE } from './system-prompt-selection'

;(
  globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }
).IS_REACT_ACT_ENVIRONMENT = true

const defaultEntry: AgentEntry = {
  id: 'default',
  name: 'Default',
  description:
    'Use for general questions and development tasks in your project.',
  logo: null,
  icon: null,
  model: null,
  skill_count: null,
  composer_placeholder: 'Example: Explain this project.',
  modified_at: '',
}

const cleanups: Array<() => void> = []
afterEach(() => {
  for (const cleanup of cleanups.splice(0)) cleanup()
})

describe('Default preselection timing', () => {
  it('commits the automatic pick before any passive effect or paint', () => {
    const calls: Array<
      [AgentProfileSnapshot | undefined, AgentProfileChangeOptions | undefined]
    > = []
    let appliedBeforeLayoutEnd: boolean | null = null
    // Layout effects run in tree order and all of them run before any
    // passive effect: a probe placed after the gallery sees the pick only
    // when the gallery applies it in a layout effect.
    function LayoutProbe() {
      useLayoutEffect(() => {
        appliedBeforeLayoutEnd = calls.length > 0
      }, [])
      return null
    }
    const container = document.createElement('div')
    document.body.appendChild(container)
    const root = createRoot(container)
    act(() =>
      root.render(
        <>
          <EmptyState
            variant="ready"
            systemPrompt={DEFAULT_SYSTEM_PROMPT_STATE}
            onSystemPromptChange={() => {}}
            onAgentProfileChange={(profile, options) =>
              calls.push([profile, options])
            }
            agentEntries={[defaultEntry]}
            preselectDefaultAgent
          />
          <LayoutProbe />
        </>,
      ),
    )
    cleanups.push(() => {
      act(() => root.unmount())
      container.remove()
    })

    expect(appliedBeforeLayoutEnd).toBe(true)
    expect(calls).toEqual([
      [
        {
          id: 'default',
          name: 'Default',
          composerPlaceholder: 'Example: Explain this project.',
        },
        { keepThinkingLevel: true },
      ],
    ])
  })
})
