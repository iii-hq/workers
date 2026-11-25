// @vitest-environment jsdom
import { act } from 'react'
import { createRoot } from 'react-dom/client'
import { describe, expect, it, vi } from 'vitest'
import { TooltipProvider } from '@/components/ui/Tooltip'
import type { PinnedModel } from '@/lib/pinned-models'
import { ModelLibraryDialog } from './ModelLibraryDialog'

vi.mock('@/lib/workers-registry', async (importOriginal) => ({
  ...(await importOriginal<typeof import('@/lib/workers-registry')>()),
  fetchRegistryProviders: async () => [
    {
      name: 'provider-deepseek',
      description: 'DeepSeek provider worker',
      version: '0.2.1',
      tags: ['provider'],
    },
    {
      name: 'provider-anthropic',
      description: 'Anthropic provider worker',
      version: '1.0.0',
      tags: ['provider'],
    },
  ],
}))

;(
  globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }
).IS_REACT_ACT_ENVIRONMENT = true

const options = [
  { id: 'anthropic::claude-opus-5', label: 'claude-opus-5' },
  { id: 'anthropic::claude-haiku-5', label: 'claude-haiku-5' },
  { id: 'github-copilot::gpt-4.1', label: 'gpt-4.1' },
]
const providers = [
  {
    id: 'anthropic',
    display_name: 'Anthropic',
    configured: true,
    available: true,
  },
  {
    id: 'github-copilot',
    display_name: 'GitHub Copilot',
    configured: true,
    available: true,
  },
  { id: 'openai', display_name: 'OpenAI', configured: false, available: true },
] as never

function render(pinned: PinnedModel[], onPinnedChange = vi.fn()) {
  const container = document.body.appendChild(document.createElement('div'))
  const root = createRoot(container)
  act(() =>
    root.render(
      <TooltipProvider>
        <ModelLibraryDialog
          open
          onOpenChange={() => {}}
          value={null}
          options={options}
          providers={providers}
          pinned={pinned}
          thinkingLevel="default"
          onPinnedChange={onPinnedChange}
        />
      </TooltipProvider>,
    ),
  )
  return {
    onPinnedChange,
    cleanup: () => {
      act(() => root.unmount())
      container.remove()
    },
  }
}

const modelRows = () =>
  [...document.body.querySelectorAll<HTMLElement>('[data-model-option]')].map(
    (row) => row.dataset.modelOption,
  )
const groups = () =>
  [...document.body.querySelectorAll<HTMLElement>('[data-provider-group]')].map(
    (group) => group.dataset.providerGroup,
  )

function search(text: string) {
  const input = document.body.querySelector<HTMLInputElement>(
    'input[aria-label="search models"]',
  )
  if (!input) throw new Error('search missing')
  act(() => {
    Object.getOwnPropertyDescriptor(
      HTMLInputElement.prototype,
      'value',
    )?.set?.call(input, text)
    input.dispatchEvent(new Event('input', { bubbles: true }))
  })
}

describe('ModelLibraryDialog', () => {
  it('search narrows models across providers by name and provider', async () => {
    const view = render([])
    try {
      expect(groups()).toEqual(['anthropic', 'github-copilot', 'openai'])
      search('opus')
      expect(modelRows()).toEqual(['anthropic::claude-opus-5'])
      expect(groups()).toEqual(['anthropic'])
      search('copilot gpt')
      expect(modelRows()).toEqual(['github-copilot::gpt-4.1'])
      // A provider still to configure matches by its name.
      search('openai')
      expect(groups()).toEqual(['openai'])
      search('nothing-like-this')
      await act(async () => {})
      expect(modelRows()).toEqual([])
      expect(document.body.textContent).toContain(
        'No model or provider matches',
      )
    } finally {
      view.cleanup()
    }
  })

  it('search finds providers to add, each with Add provider', async () => {
    const view = render([])
    try {
      search('deep')
      await act(async () => {})
      const section = document.body.querySelector(
        '[data-registry-provider="provider-deepseek"]',
      )
      expect(section?.textContent).toContain('DeepSeek')
      // The accessible name contains the visible label (WCAG 2.5.3).
      expect(
        section?.querySelector('button[aria-label="Add provider DeepSeek"]')
          ?.textContent,
      ).toBe('Add provider')
      // An installed provider is not offered again.
      search('anthropic')
      await act(async () => {})
      expect(document.body.querySelector('[data-registry-provider]')).toBeNull()
      expect(document.body.textContent).not.toContain(
        'No model or provider matches',
      )
    } finally {
      view.cleanup()
    }
  })

  it('offers Add provider beside the search', () => {
    const view = render([])
    try {
      const add = document.body.querySelector<HTMLButtonElement>(
        'button[aria-label="Add a provider"]',
      )
      if (!add) throw new Error('add provider missing')
      act(() => add.click())
      expect(document.body.textContent).toContain('Add a provider')
      expect(
        document.body.querySelector('input[aria-label="search models"]'),
      ).toBeNull()
    } finally {
      view.cleanup()
    }
  })
})
