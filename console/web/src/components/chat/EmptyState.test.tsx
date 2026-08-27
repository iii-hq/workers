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

  it('keeps session prompt and selected skills compact beneath the question', () => {
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
        skills={['design', 'linear-workflow']}
        onSkillsChange={() => {}}
        agentEntries={agents}
      />,
    )

    expect(html).toContain('aria-label="session setup"')
    expect(html).toContain('Configure manually')
    expect(html).toContain('aria-expanded="true"')
    expect(html).toContain('data-open="true"')
    expect(html).toContain('t-acc-panel')
    expect(html).toContain('System prompt')
    expect(html).toContain('reviewer')
    expect(html).toContain('items-baseline')
    expect(html).not.toContain('Extending')
    expect(html).not.toContain('Overriding')
    expect(html).toContain('border-dashed')
    expect(html).toContain('Add skills')
    expect(html).toContain('remove design from this session')
    expect(html).toContain('remove linear-workflow from this session')
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

    expect(html).toContain('Choose an agent profile')
    expect(html).toContain('aria-expanded="false"')
    expect(html).toContain('data-open="false"')
    expect(html).toContain('Use Engineer agent profile')
    expect(html).toContain('aria-pressed="true"')
    expect(html).toContain('Builds and reviews production code.')
    expect(html).toContain('Use Researcher agent profile')
    expect(html).toContain('active-subagent-chip')
    expect(html).toContain('data-color="blue"')
    expect(html).not.toContain('Sub-agent')
    expect(html).not.toContain('ActivityMetadata')
  })
})
