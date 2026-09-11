import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it } from 'vitest'
import { TooltipProvider } from '@/components/ui/Tooltip'
import type { ProviderListEntry } from '@/lib/models-catalog'
import type { ModelOption } from '@/types/chat'
import { ModelPickerPanel } from './ModelPicker'

const MARK =
  '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path d="M0 0h24v24H0z"/></svg>'

const PROVIDERS: ProviderListEntry[] = [
  {
    id: 'anthropic',
    display_name: 'Anthropic',
    supports_model_listing: true,
    credential_env_var: 'ANTHROPIC_API_KEY',
    configured: true,
    available: true,
    icon_svg: MARK,
  },
  {
    id: 'openai',
    display_name: 'OpenAI',
    supports_model_listing: true,
    credential_env_var: 'OPENAI_API_KEY',
    configured: true,
    available: true,
  },
]

const OPTIONS: ModelOption[] = [
  { id: 'anthropic::claude-opus-4-7', label: 'claude opus 4.7' },
  {
    id: 'openai::gpt-5',
    label: 'gpt-5',
    supportsThinking: true,
    reasoningEfforts: [
      { effort: 'low', description: 'quick' },
      { effort: 'medium', description: 'balanced' },
      { effort: 'high', description: 'deep' },
    ],
  },
  { id: 'openai::gpt-4.1', label: 'gpt-4.1' },
]

function render(ui: React.ReactElement): string {
  return renderToStaticMarkup(<TooltipProvider>{ui}</TooltipProvider>)
}

describe('ModelPickerPanel', () => {
  it('renders one rail glyph per provider group plus the add affordance', () => {
    const html = render(
      <ModelPickerPanel
        value="openai::gpt-5"
        options={OPTIONS}
        providers={PROVIDERS}
        thinkingLevel="medium"
        onChange={() => {}}
        onThinkingLevelChange={() => {}}
        onAddProvider={() => {}}
      />,
    )
    expect(html).toContain('aria-label="providers"')
    expect(html).toContain('data-provider-rail="anthropic"')
    expect(html).toContain('data-provider-rail="openai"')
    expect(html).toContain('aria-label="Add a provider"')
    // The declared mark paints as a mask; the provider without one gets its initial.
    expect(html).toContain('data-provider-icon="mark"')
    expect(html).toContain('--provider-icon-mask:url(&quot;data:image/svg+xml')
    expect(html).toMatch(/data-provider-icon="initial"[^>]*>O</)
    expect(html).toContain('data-provider-group="anthropic"')
    expect(html).toContain('data-provider-group="openai"')
  })

  it('marks the selection with the stronger wash and never draws a ring on rows', () => {
    const html = render(
      <ModelPickerPanel
        value="openai::gpt-5"
        options={OPTIONS}
        providers={PROVIDERS}
        thinkingLevel="medium"
        onChange={() => {}}
        onThinkingLevelChange={() => {}}
      />,
    )
    const rows = html.match(/<button[^>]*data-model-option=[^>]*>/g) ?? []
    expect(rows).toHaveLength(3)
    for (const row of rows) {
      const selected = row.includes('aria-pressed="true"')
      // The selected row keeps its wash under the pointer; every other row
      // takes the lighter hover wash. Neither draws a ring.
      expect(row).toContain(
        selected ? 'hover:bg-surface-selected' : 'hover:bg-surface-hover',
      )
      expect(row.includes('bg-surface-selected')).toBe(selected)
      expect(row).not.toContain('ring-rule-focus')
    }
  })

  it('shows the effort slider for the selected model and a quiet note otherwise', () => {
    const withLevels = render(
      <ModelPickerPanel
        value="openai::gpt-5"
        options={OPTIONS}
        providers={PROVIDERS}
        thinkingLevel="high"
        onChange={() => {}}
        onThinkingLevelChange={() => {}}
      />,
    )
    expect(withLevels).toContain('type="range"')
    expect(withLevels).toContain('aria-valuetext="high"')
    // default + low + medium + high → high sits at the end of the ramp.
    expect(withLevels).toContain('--effort-ratio:1')

    const fixed = render(
      <ModelPickerPanel
        value="openai::gpt-4.1"
        options={OPTIONS}
        providers={PROVIDERS}
        thinkingLevel="default"
        onChange={() => {}}
        onThinkingLevelChange={() => {}}
      />,
    )
    expect(fixed).not.toContain('type="range"')
    expect(fixed).toContain('Fixed for this model')

    const hidden = render(
      <ModelPickerPanel
        value="openai::gpt-5"
        options={OPTIONS}
        providers={PROVIDERS}
        thinkingLevel="high"
        onChange={() => {}}
        onThinkingLevelChange={() => {}}
        showReasoningEffort={false}
      />,
    )
    expect(hidden).not.toContain('Reasoning effort')
  })

  it('invites adding a provider when the catalog is empty', () => {
    const html = render(
      <ModelPickerPanel
        value={null}
        options={[]}
        providers={[]}
        thinkingLevel="default"
        onChange={() => {}}
        onThinkingLevelChange={() => {}}
        onAddProvider={() => {}}
      />,
    )
    expect(html).toContain('Add one from the registry')
    expect(html).toContain('aria-label="Add a provider"')
    expect(html).toContain('Choose a model to adjust')
  })
})
