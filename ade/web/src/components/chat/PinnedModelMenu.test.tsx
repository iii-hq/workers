// @vitest-environment jsdom
import { act } from 'react'
import { createRoot } from 'react-dom/client'
import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it, vi } from 'vitest'
import { TooltipProvider } from '@/components/ui/Tooltip'
import type { ProviderListEntry } from '@/lib/models-catalog'
import type { PinnedModel } from '@/lib/pinned-models'
import type { ModelOption } from '@/types/chat'
import { favoritesFullNotice, PinnedModelMenu } from './PinnedModelMenu'

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
  it('opens on the favorites, every one starred, with the effort slider and its stops', () => {
    const html = renderToStaticMarkup(menu())
    expect(html).toContain('data-model-menu-view="favorites"')
    const rows = html.match(/<button[^>]*data-model-option=[^>]*>/g) ?? []
    expect(rows).toHaveLength(2)
    expect(html.match(/data-model-star="on"/g)).toHaveLength(2)
    expect(html).not.toContain('data-model-star="off"')
    // No provider groups until a filter is typed.
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
    // The footer: All models (the library dialog) and Add provider.
    expect(html).toMatch(
      /<button[^>]*>[^<]*<svg[^>]*>.*?<\/svg>All models<\/button>/,
    )
    expect(html).toContain('aria-label="Add a provider"')
    expect(html).not.toContain('Cycle effort')
  })

  it('a typed filter lists every matching model grouped by provider, with a star per row', () => {
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
      expect(container.querySelectorAll('[data-model-star="on"]')).toHaveLength(
        2,
      )
      expect(
        container.querySelectorAll('[data-model-star="off"]'),
      ).toHaveLength(4)
      expect((html.match(/>Configure</g) ?? []).length).toBe(2)
      // Selection and starring are separate buttons: the star never sits
      // inside the row's select button.
      for (const row of container.querySelectorAll('[data-model-option]')) {
        expect(row.querySelector('[data-model-star]')).toBeNull()
      }
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

  it('refuses a sixth star with a notice instead of swapping one out', async () => {
    const onPinnedChange = vi.fn()
    const container = document.body.appendChild(document.createElement('div'))
    const root = createRoot(container)
    try {
      await act(async () => root.render(menu({ pinned: FIVE, onPinnedChange })))
      // Every id carries the provider separator, so ':' matches the whole catalog.
      typeFilter(container, ':')
      const star = container.querySelector<HTMLButtonElement>(
        'button[aria-label="Star GPT-4.1 Mini"]',
      )
      if (!star) throw new Error('hollow star missing')
      expect(star.title).toContain('Favorites are full')
      await act(async () => star.click())
      expect(onPinnedChange).not.toHaveBeenCalled()
      const notice = container.querySelector('[data-favorites-limit-notice]')
      expect(notice?.getAttribute('role')).toBe('status')
      expect(notice?.textContent).toBe(favoritesFullNotice('GPT-4.1 Mini'))

      // Unstarring is always allowed, and clears the notice.
      const starred = container.querySelector<HTMLButtonElement>(
        'button[aria-label="Unstar Opus 4.7"]',
      )
      if (!starred) throw new Error('filled star missing')
      await act(async () => starred.click())
      expect(onPinnedChange).toHaveBeenCalledWith(
        FIVE.filter((entry) => entry.id !== 'anthropic::claude-opus-4-7'),
      )
      expect(
        container.querySelector('[data-favorites-limit-notice]'),
      ).toBeNull()
    } finally {
      act(() => root.unmount())
      container.remove()
    }
  })

  it('stars a model at its live effort when it is the current one, else at the default', async () => {
    const onPinnedChange = vi.fn()
    const container = document.body.appendChild(document.createElement('div'))
    const root = createRoot(container)
    try {
      await act(async () =>
        root.render(
          menu({
            pinned: [],
            thinkingLevel: 'high',
            onPinnedChange,
          }),
        ),
      )
      // Every id carries the provider separator, so ':' matches the whole catalog.
      typeFilter(container, ':')
      const current = container.querySelector<HTMLButtonElement>(
        'button[aria-label="Star GPT-5"]',
      )
      if (!current) throw new Error('star missing')
      await act(async () => current.click())
      expect(onPinnedChange).toHaveBeenLastCalledWith([
        { id: 'openai::gpt-5', effort: 'high' },
      ])
      const other = container.querySelector<HTMLButtonElement>(
        'button[aria-label="Star Haiku 4.5"]',
      )
      if (!other) throw new Error('star missing')
      await act(async () => other.click())
      expect(onPinnedChange).toHaveBeenLastCalledWith([
        { id: 'anthropic::claude-haiku-4-5', effort: 'default' },
      ])
    } finally {
      act(() => root.unmount())
      container.remove()
    }
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
