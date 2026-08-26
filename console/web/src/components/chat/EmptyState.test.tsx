import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it } from 'vitest'
import { EmptyState } from './EmptyState'
import { DEFAULT_SYSTEM_PROMPT_STATE } from './system-prompt-selection'

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
      />,
    )

    expect(html).toContain('aria-label="session setup"')
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
})
