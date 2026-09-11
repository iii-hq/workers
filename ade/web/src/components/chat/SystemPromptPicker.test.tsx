import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it } from 'vitest'
import { StrategyToggle, SystemPromptPickerPanel } from './SystemPromptPicker'
import { DEFAULT_SYSTEM_PROMPT_STATE } from './system-prompt-selection'

describe('system prompt strategy', () => {
  it('renders the selected strategy as an inline dropdown', () => {
    const strategies = renderToStaticMarkup(
      <StrategyToggle
        value={{
          ...DEFAULT_SYSTEM_PROMPT_STATE,
          choice: { named: 'reviewer' },
        }}
        onChange={() => {}}
        appearance="inline"
      />,
    )

    expect(strategies).toContain('role="combobox"')
    expect(strategies).toContain('Extending')
    expect(strategies).toContain('border-dashed')
    expect(strategies).not.toContain('Enrich')
  })

  it('renders the mobile sheet options as native radios', () => {
    const options = renderToStaticMarkup(
      <SystemPromptPickerPanel
        value={DEFAULT_SYSTEM_PROMPT_STATE}
        entries={[
          {
            name: 'reviewer',
            description: 'Review the proposed changes',
            modified_at: '2026-08-26T00:00:00Z',
          },
        ]}
        allowCustom
        onSelect={() => {}}
      />,
    )

    expect(options).toContain('type="radio"')
    expect(options).toContain('Review the proposed changes')
    expect(options).toContain('Custom…')
    expect(options).toContain('aria-hidden="true"')
  })
})
