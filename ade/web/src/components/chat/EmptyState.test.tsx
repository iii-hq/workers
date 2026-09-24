import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it } from 'vitest'
import { EmptyState } from './EmptyState'
import {
  AGENT_CHOICE_PREFIX,
  DEFAULT_SYSTEM_PROMPT_STATE,
} from './system-prompt-selection'

const agents = [
  {
    id: 'engineer',
    name: 'Engineer',
    description: 'Builds and reviews production code.',
    logo: null,
    icon: 'code',
    color: 'blue',
    model: null,
    skill_count: 2,
    modified_at: '2026-08-27T00:00:00.000Z',
  },
  {
    id: 'researcher',
    name: 'Researcher',
    description: 'Finds evidence and summarizes it.',
    logo: null,
    icon: 'search',
    color: 'teal',
    model: null,
    skill_count: 1,
    modified_at: '2026-08-27T00:00:00.000Z',
  },
]

describe('EmptyState', () => {
  it('shrinks and scrolls while keeping short content centered', () => {
    const html = renderToStaticMarkup(
      <EmptyState variant="ready" workingDir="/workspace/console" />,
    )

    expect(html).toContain('min-h-0 overflow-y-auto')
    expect(html).toContain('my-auto')
    expect(html).toContain('size-16')
    expect(html).toContain('What should we build in')
    expect(html).toContain('console')
  })

  it('keeps the agent gallery visible for an existing manual configuration', () => {
    const html = renderToStaticMarkup(
      <EmptyState
        variant="ready"
        workingDir="/workspace/console"
        onWorkingDirChange={() => {}}
        systemPrompt={{
          ...DEFAULT_SYSTEM_PROMPT_STATE,
          choice: { named: 'reviewer' },
        }}
        onSystemPromptChange={() => {}}
        agentEntries={agents}
      />,
    )

    expect(html).toContain('aria-label="session setup"')
    expect(html).toContain('Use Engineer agent profile')
    expect(html).toContain('Use Researcher agent profile')
    expect(html).toContain('Configure an agent manually in the profile editor')
    expect(html).not.toContain('Configure manually')
    expect(html).not.toContain('System prompt')
    expect(html).not.toContain('remove design from this session')
    expect(html).not.toContain('remove linear-workflow from this session')
  })

  it('shows agent cards by default with only identity content', () => {
    const html = renderToStaticMarkup(
      <EmptyState
        variant="ready"
        workingDir="/workspace/console"
        systemPrompt={{
          ...DEFAULT_SYSTEM_PROMPT_STATE,
          choice: { named: `${AGENT_CHOICE_PREFIX}engineer` },
        }}
        onSystemPromptChange={() => {}}
        agentEntries={agents}
      />,
    )

    expect(html).toContain('Use Engineer agent profile')
    expect(html).toContain('aria-pressed="true"')
    expect(html).toContain('Builds and reviews production code.')
    expect(html).toContain('Use Researcher agent profile')
    expect(html).toContain('active-subagent-chip')
    expect(html).toContain('data-color="blue"')
    expect(html).toContain('Configure an agent manually in the profile editor')
    expect(html).toContain('@lg:grid-cols-2')
    expect(html).toContain('@3xl:grid-cols-3')
    expect(html).toContain('@lg:min-h-40')
    expect(html).not.toContain('h-full min-h-40')
    expect(html).toContain('flex min-w-0 items-center gap-3 @lg:block')
    expect(html).toContain('text-ink @lg:hidden')
    expect(html).toContain('hidden font-sans text-sm')
    expect(html).toContain('@lg:block')
    expect(html).toContain('size-4 h-lh shrink-0')
    expect(html).toContain('Open profile editor')
    expect(html).toContain('flex items-center gap-1.5')
    expect(html).not.toContain('Sub-agent')
    expect(html).not.toContain('ActivityMetadata')
    expect(html).not.toContain('Configure manually')
  })

  it('uses the frozen session profile to keep the selected card identified', () => {
    const html = renderToStaticMarkup(
      <EmptyState
        variant="ready"
        systemPrompt={DEFAULT_SYSTEM_PROMPT_STATE}
        onSystemPromptChange={() => {}}
        agentEntries={agents}
        agentProfile={{
          id: 'researcher',
          name: 'Researcher',
          model: 'openai-codex::codex/gpt-5.6-sol',
          reasoningEffort: 'high',
          icon: 'search',
          color: 'teal',
        }}
      />,
    )

    expect(html).toContain('Use Researcher agent profile')
    expect(html).toMatch(/Use Researcher agent profile[^>]*aria-pressed="true"/)
  })

  it('offers the manual profile editor as a secondary, predictable action', () => {
    const html = renderToStaticMarkup(
      <EmptyState
        variant="ready"
        systemPrompt={DEFAULT_SYSTEM_PROMPT_STATE}
        onSystemPromptChange={() => {}}
        agentEntries={[]}
      />,
    )

    expect(html).toContain('Configure an agent manually')
    expect(html).toContain(
      'Opens a form to set instructions, a model, and skills yourself.',
    )
    expect(html).toContain('Open profile editor')
    expect(html).not.toContain('Create a new agent')
    expect(html).toContain('bg-surface')
    expect(html).not.toContain('shadow-raised')
  })

  const withDefault = [
    ...agents,
    {
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
    },
  ]

  it('marks Default as the selected card of an untouched local draft', () => {
    const html = renderToStaticMarkup(
      <EmptyState
        variant="ready"
        systemPrompt={DEFAULT_SYSTEM_PROMPT_STATE}
        onSystemPromptChange={() => {}}
        onAgentProfileChange={() => {}}
        agentEntries={withDefault}
        preselectDefaultAgent
      />,
    )

    expect(html).toMatch(/Use Default agent profile[^>]*aria-pressed="true"/)
    expect(html).toMatch(/Use Engineer agent profile[^>]*aria-pressed="false"/)
  })

  it('leaves a session created elsewhere unselected', () => {
    // An empty server session (created by a trigger, the API, another
    // client) is not a local draft: opening it must not pick a profile.
    const html = renderToStaticMarkup(
      <EmptyState
        variant="ready"
        systemPrompt={DEFAULT_SYSTEM_PROMPT_STATE}
        onSystemPromptChange={() => {}}
        onAgentProfileChange={() => {}}
        agentEntries={withDefault}
      />,
    )
    expect(html).not.toContain('aria-pressed="true"')
  })

  it('does not preselect Default over a manual or explicit choice', () => {
    const manual = renderToStaticMarkup(
      <EmptyState
        variant="ready"
        systemPrompt={{
          ...DEFAULT_SYSTEM_PROMPT_STATE,
          choice: { named: 'reviewer' },
        }}
        onSystemPromptChange={() => {}}
        agentEntries={withDefault}
        preselectDefaultAgent
      />,
    )
    expect(manual).toMatch(/Use Default agent profile[^>]*aria-pressed="false"/)

    const explicit = renderToStaticMarkup(
      <EmptyState
        variant="ready"
        systemPrompt={DEFAULT_SYSTEM_PROMPT_STATE}
        onSystemPromptChange={() => {}}
        agentEntries={withDefault}
        agentProfile={{ id: 'engineer', name: 'Engineer' }}
        preselectDefaultAgent
      />,
    )
    expect(explicit).toMatch(
      /Use Default agent profile[^>]*aria-pressed="false"/,
    )
    expect(explicit).toMatch(
      /Use Engineer agent profile[^>]*aria-pressed="true"/,
    )
  })
})
