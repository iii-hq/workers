import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it } from 'vitest'
import { EmptyState } from './EmptyState'
import { StrategyToggle } from './SystemPromptPicker'
import { DEFAULT_SYSTEM_PROMPT_STATE } from './system-prompt-selection'

describe('new-session system prompt', () => {
  it('explains the default and makes both named-prompt strategies explicit', () => {
    const ready = renderToStaticMarkup(
      <EmptyState
        variant="ready"
        systemPrompt={DEFAULT_SYSTEM_PROMPT_STATE}
        onSystemPromptChange={() => {}}
      />,
    )
    const strategies = renderToStaticMarkup(
      <StrategyToggle
        value={{
          ...DEFAULT_SYSTEM_PROMPT_STATE,
          choice: { named: 'reviewer' },
        }}
        onChange={() => {}}
      />,
    )

    expect(ready).toContain('System prompt')
    expect(ready).toContain('Default uses the provider&#x27;s built-in prompt')
    expect(ready).toContain('Locked after your first message')
    expect(strategies).toContain('aria-pressed="true"')
    expect(strategies).toContain('Enrich')
    expect(strategies).toContain('Replace')
  })
})
