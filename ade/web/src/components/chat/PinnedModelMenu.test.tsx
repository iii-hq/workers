// @vitest-environment jsdom
import { act } from 'react'
import { createRoot } from 'react-dom/client'
import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it, vi } from 'vitest'
import { TooltipProvider } from '@/components/ui/Tooltip'
import type { ProviderListEntry } from '@/lib/models-catalog'
import type { PinnedModel } from '@/lib/pinned-models'
import type { ModelOption } from '@/types/chat'
import { PinnedModelMenu } from './PinnedModelMenu'

;(
  globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }
).IS_REACT_ACT_ENVIRONMENT = true

// jsdom does not implement scrolling; the menu scrolls the active row.
Element.prototype.scrollIntoView = () => {}

const PROVIDERS: ProviderListEntry[] = [
  {
    id: 'anthropic',
    display_name: 'Anthropic',
    supports_model_listing: true,
    credential_env_var: 'ANTHROPIC_API_KEY',
    configured: true,
    available: true,
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

const EFFORTS = [
  { effort: 'default', description: 'use the model default' },
  { effort: 'low', description: 'quick' },
  { effort: 'medium', description: 'balanced' },
  { effort: 'high', description: 'deep' },
]

const OPTIONS: ModelOption[] = [
  { id: 'anthropic::claude-opus-4-7', label: 'claude opus 4.7' },
  { id: 'anthropic::claude-sonnet-4-5', label: 'claude sonnet 4.5' },
  { id: 'anthropic::claude-haiku-4-5', label: 'claude haiku 4.5' },
  {
    id: 'openai::gpt-5',
    label: 'gpt-5',
    supportsThinking: true,
    reasoningEfforts: EFFORTS.slice(1),
  },
  { id: 'openai::gpt-4.1', label: 'gpt-4.1' },
  { id: 'openai::gpt-4.1-mini', label: 'gpt-4.1-mini' },
]

const FIVE: PinnedModel[] = [
  { id: 'openai::gpt-5', effort: 'medium' },
  { id: 'anthropic::claude-opus-4-7', effort: 'default' },
  { id: 'anthropic::claude-sonnet-4-5', effort: 'default' },
  { id: 'anthropic::claude-haiku-4-5', effort: 'default' },
  { id: 'openai::gpt-4.1', effort: 'default' },
]

function menu(
  overrides: Partial<Parameters<typeof PinnedModelMenu>[0]> = {},
): React.ReactElement {
  return (
    <TooltipProvider>
      <PinnedModelMenu
        value="openai::gpt-5"
        options={OPTIONS}
        providers={PROVIDERS}
        pinned={FIVE.slice(0, 2)}
        thinkingLevel="medium"
        efforts={EFFORTS}
        onSelect={() => {}}
        onThinkingLevelChange={() => {}}
        onPinnedChange={() => {}}
        onConfigureProvider={() => {}}
        onAddProvider={() => {}}
        onEdit={() => {}}
        autoFocusSearch={false}
        {...overrides}
      />
    </TooltipProvider>
  )
}

function typeFilter(container: HTMLElement, text: string) {
  const input = container.querySelector<HTMLInputElement>(
    'input[role="combobox"]',
  )
  if (!input) throw new Error('search missing')
  act(() => {
    const setter = Object.getOwnPropertyDescriptor(
      HTMLInputElement.prototype,
      'value',
    )?.set
    setter?.call(input, text)
    input.dispatchEvent(new Event('input', { bubbles: true }))
  })
}

describe('PinnedModelMenu', () => {
  it('opens on the favorites, no stars, All models folded, with the effort slider and its stops', () => {
    const html = renderToStaticMarkup(menu())
    expect(html).toContain('data-model-menu-view="favorites"')
    const rows = html.match(/<button[^>]*data-model-option=[^>]*>/g) ?? []
    expect(rows).toHaveLength(2)
    expect(html).not.toContain('data-model-star')
    // All models sits folded under the favorites, counting the catalog;
    // no provider groups until it is opened or a filter is typed.
    expect(html).toContain('data-all-models="closed"')
    expect(html).toMatch(/aria-expanded="false"[^>]*>.*?All models.*?>6</)
    expect(html).not.toContain('data-provider-group=')
    // The effort is a slider, stacked, naming the model's own levels.
    expect(html).toContain('type="range"')
    expect(html).toContain('reasoning-effort--stacked')
    expect(html).toContain('aria-valuetext="medium"')
    const stops = html.match(/aria-label="[^"]+ reasoning effort"/g) ?? []
    expect(stops).toEqual([
      'aria-label="Default reasoning effort"',
      'aria-label="Low reasoning effort"',
      'aria-label="Medium reasoning effort"',
      'aria-label="High reasoning effort"',
    ])
    // The footer: Manage models (the library dialog) and Add provider.
    expect(html).toMatch(
      /<button[^>]*>[^<]*<svg[^>]*>.*?<\/svg>Manage models<\/button>/,
    )
    expect(html).toContain('aria-label="Add a provider"')
    expect(html).not.toContain('Cycle effort')
  })

  it('a typed filter lists every matching model grouped by provider', () => {
    const container = document.body.appendChild(document.createElement('div'))
    const root = createRoot(container)
    try {
      act(() => root.render(menu()))
      // Every id carries the provider separator, so ':' matches the whole catalog.
      typeFilter(container, ':')
      const html = container.innerHTML
      expect(html).toContain('data-model-menu-view="catalog"')
      expect(html).toContain('data-provider-group="anthropic"')
      expect(html).toContain('data-provider-group="openai"')
      expect(html).toContain('aria-label="Anthropic"')
      expect(html).toContain('aria-label="OpenAI"')
      expect(container.querySelectorAll('[data-model-option]')).toHaveLength(
        OPTIONS.length,
      )
      expect(container.querySelector('[data-model-star]')).toBeNull()
      // The folded section gives way to the results while searching.
      expect(container.querySelector('[data-all-models]')).toBeNull()
      expect((html.match(/>Configure</g) ?? []).length).toBe(2)
    } finally {
      act(() => root.unmount())
      container.remove()
    }
  })

  it('says the effort is fixed for a model with one level', () => {
    const html = renderToStaticMarkup(
      menu({ value: 'openai::gpt-4.1', efforts: [EFFORTS[0]] }),
    )
    expect(html).not.toContain('type="range"')
    expect(html).toContain('Fixed for this model')
  })

  it('All models expands to the whole catalog, grouped, and a row there selects', async () => {
    const onSelect = vi.fn()
    const container = document.body.appendChild(document.createElement('div'))
    const root = createRoot(container)
    try {
      await act(async () => root.render(menu({ onSelect })))
      const toggle = container.querySelector<HTMLButtonElement>(
        '[data-all-models] > button[aria-expanded]',
      )
      if (!toggle) throw new Error('All models toggle missing')
      await act(async () => toggle.click())
      expect(toggle.getAttribute('aria-expanded')).toBe('true')
      const section = container.querySelector('[data-all-models="open"]')
      expect(section).not.toBeNull()
      expect(
        section?.querySelector('[data-provider-group="anthropic"]'),
      ).not.toBeNull()
      expect(
        section?.querySelector('[data-provider-group="openai"]'),
      ).not.toBeNull()
      expect(section?.querySelectorAll('[data-model-option]')).toHaveLength(
        OPTIONS.length,
      )
      // A favorite also listed under All models is two rows with two ids.
      const ids = [...container.querySelectorAll('[data-model-option]')].map(
        (row) => row.id,
      )
      expect(new Set(ids).size).toBe(ids.length)
      expect(container.querySelector('[data-model-star]')).toBeNull()

      const haiku = section?.querySelector<HTMLButtonElement>(
        '[data-model-option="anthropic::claude-haiku-4-5"]',
      )
      if (!haiku) throw new Error('catalog row missing')
      await act(async () => haiku.click())
      expect(onSelect).toHaveBeenCalledWith(
        'anthropic::claude-haiku-4-5',
        undefined,
      )

      // Collapsing folds the catalog away again.
      await act(async () => toggle.click())
      expect(
        container.querySelector('[data-all-models="closed"]'),
      ).not.toBeNull()
      expect(container.querySelectorAll('[data-model-option]')).toHaveLength(2)
    } finally {
      act(() => root.unmount())
      container.remove()
    }
  })

  it('arrows walk from the favorites into an open All models, and Enter picks', async () => {
    const onSelect = vi.fn()
    const container = document.body.appendChild(document.createElement('div'))
    const root = createRoot(container)
    try {
      await act(async () => root.render(menu({ onSelect })))
      const toggle = container.querySelector<HTMLButtonElement>(
        '[data-all-models] > button[aria-expanded]',
      )
      await act(async () => toggle?.click())
      const input = container.querySelector<HTMLInputElement>(
        'input[role="combobox"]',
      )
      if (!input) throw new Error('search missing')
      const press = (key: string) =>
        act(async () => {
          input.dispatchEvent(
            new KeyboardEvent('keydown', { key, bubbles: true }),
          )
        })
      // Starts on the current model (GPT-5, the first favorite); two steps
      // pass the second favorite and land on All models' first row.
      await press('ArrowDown')
      await press('ArrowDown')
      expect(input.getAttribute('aria-activedescendant')).toBe(
        'model-option-all:anthropic::claude-opus-4-7',
      )
      await press('Enter')
      expect(onSelect).toHaveBeenCalledWith(
        'anthropic::claude-opus-4-7',
        undefined,
      )
    } finally {
      act(() => root.unmount())
      container.remove()
    }
  })

  it('a favorite selects at the effort it was pinned with', async () => {
    const onSelect = vi.fn()
    const container = document.body.appendChild(document.createElement('div'))
    const root = createRoot(container)
    try {
      await act(async () =>
        root.render(menu({ onSelect, pinned: FIVE.slice(0, 2) })),
      )
      const opus = container.querySelector<HTMLButtonElement>(
        '[data-model-row="fav:anthropic::claude-opus-4-7"]',
      )
      if (!opus) throw new Error('favorite row missing')
      await act(async () => opus.click())
      expect(onSelect).toHaveBeenCalledWith(
        'anthropic::claude-opus-4-7',
        'default',
      )
    } finally {
      act(() => root.unmount())
      container.remove()
    }
  })

  it('opens All models from the start when there are no favorites', () => {
    const html = renderToStaticMarkup(menu({ value: null, pinned: [] }))
    expect(html).toContain('No favorites yet. Pin up to five in Manage models.')
    expect(html).toContain('data-all-models="open"')
    const rows = html.match(/<button[^>]*data-model-option=[^>]*>/g) ?? []
    expect(rows).toHaveLength(OPTIONS.length)
  })

  it('a typed filter searches the whole catalog, grouped', () => {
    const container = document.body.appendChild(document.createElement('div'))
    const root = createRoot(container)
    try {
      act(() => root.render(menu()))
      typeFilter(container, 'mini')
      expect(
        container.querySelector('[data-model-menu-view="catalog"]'),
      ).not.toBeNull()
      const rows = container.querySelectorAll('[data-model-option]')
      expect(rows).toHaveLength(1)
      expect(rows[0].getAttribute('data-model-option')).toBe(
        'openai::gpt-4.1-mini',
      )
      expect(
        container.querySelector('[data-provider-group="openai"]'),
      ).not.toBeNull()
      expect(
        container.querySelector('[data-provider-group="anthropic"]'),
      ).toBeNull()
    } finally {
      act(() => root.unmount())
      container.remove()
    }
  })
})
