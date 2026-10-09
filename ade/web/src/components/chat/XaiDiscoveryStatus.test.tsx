// @vitest-environment jsdom
import { act } from 'react'
import { createRoot } from 'react-dom/client'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { TooltipProvider } from '@/components/ui/Tooltip'
import {
  type DiscoveryStatus,
  type ProviderListEntry,
  parseProviderList,
} from '@/lib/models-catalog'
import { ModelPickerPanel } from './ModelPicker'

const mocks = vi.hoisted(() => ({
  refresh: vi.fn(async (_id?: string) => {}),
  context: {} as Record<string, unknown>,
}))
vi.mock('@/lib/conversations-context', () => ({
  useConversationsCtxOptional: () => mocks.context,
}))
vi.mock('@/lib/iii-client', () => ({
  getIiiClient: async () => ({
    trigger: async () => ({
      models: [{ id: 'voice', provider: 'speech', modalities: ['audio'] }],
    }),
  }),
}))
;(
  globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }
).IS_REACT_ACT_ENVIRONMENT = true
Element.prototype.scrollIntoView = () => {}
const billing: DiscoveryStatus = {
  outcome: 'billing',
  http_status: 403,
  code: 'permission-denied',
  stale: false,
  checked_at_ms: 1,
}
const xai = (discovery = billing): ProviderListEntry => ({
  id: 'xai',
  display_name: 'xAI',
  available: true,
  configured: true,
  supports_model_listing: true,
  discovery,
})
const options = [
  { id: 'xai::grok-4', label: 'grok-4' },
  { id: 'openai::gpt-5', label: 'gpt-5' },
]
const container = document.createElement('div')
document.body.append(container)
let root = createRoot(container)
afterEach(() => {
  act(() => root.unmount())
  root = createRoot(container)
  vi.clearAllMocks()
})

async function render(providers: ProviderListEntry[], onChange = vi.fn()) {
  mocks.context = { refreshModels: mocks.refresh, presentProviders: providers }
  await act(async () =>
    root.render(
      <TooltipProvider>
        <ModelPickerPanel
          value="xai::grok-4"
          options={options}
          providers={providers}
          thinkingLevel="default"
          onChange={onChange}
          onThinkingLevelChange={() => {}}
        />
      </TooltipProvider>,
    ),
  )
  return onChange
}

describe('xAI discovery feedback in the existing picker', () => {
  it('shows billing, fixed safe link and collapsed details; disables only xAI and preserves selection', async () => {
    const onChange = await render([xai()])
    expect(container.textContent).toContain('Credits or spending limit')
    expect(container.querySelector('details')?.open).toBe(false)
    expect(container.querySelector('details')?.textContent).toContain(
      'HTTP 403 / permission-denied',
    )
    const link = container.querySelector('a')
    if (!link) throw new Error('Missing safe link')
    expect(link.href).toBe('https://console.x.ai/')
    expect(link.rel).toContain('noopener')
    const blocked = container.querySelector<HTMLButtonElement>(
      '[data-model-option="xai::grok-4"]',
    )
    if (!blocked) throw new Error('Missing xAI option')
    expect(blocked.disabled).toBe(true)
    expect(blocked.getAttribute('aria-pressed')).toBe('true')
    const other = container.querySelector<HTMLButtonElement>(
      '[data-model-option="openai::gpt-5"]',
    )
    if (!other) throw new Error('Missing other provider')
    expect(other.disabled).toBe(false)
    await act(async () => blocked.click())
    expect(onChange).not.toHaveBeenCalled()
    await act(async () => other.click())
    expect(onChange).toHaveBeenCalledWith('openai::gpt-5', 'default')
  })

  it('retries only xAI, stays open while pending, clears warning and announces recovery', async () => {
    let finish!: () => void
    mocks.refresh.mockImplementationOnce(
      () =>
        new Promise<void>((resolve) => {
          finish = resolve
        }),
    )
    const onChange = await render([xai()])
    const retry = [...container.querySelectorAll('button')].find(
      (b) => b.textContent === 'Check again',
    )
    if (!retry) throw new Error('Missing retry')
    await act(async () => retry.click())
    expect(mocks.refresh).toHaveBeenCalledWith('xai')
    expect(retry.disabled).toBe(true)
    expect(retry.textContent).toBe('Checking…')
    expect(
      container.querySelector('[data-provider-group="xai"]'),
    ).not.toBeNull()
    await render(
      [xai({ outcome: 'success', stale: false, checked_at_ms: 2 })],
      onChange,
    )
    await act(async () => finish())
    expect(container.textContent).not.toContain('Credits or spending limit')
    expect(container.textContent).toContain('xAI models updated.')
    expect(
      container.querySelector<HTMLButtonElement>(
        '[data-model-option="xai::grok-4"]',
      )?.disabled,
    ).toBe(false)
    expect(onChange).not.toHaveBeenCalled()
  })

  it('distinguishes truly empty discovery and transient stale catalog', async () => {
    await render([xai({ outcome: 'empty', stale: false, checked_at_ms: 3 })])
    expect(container.textContent).toContain('The lookup completed successfully')
    expect(container.textContent).not.toContain('No models with this key')
    await render([
      xai({ outcome: 'unavailable', stale: true, checked_at_ms: 4 }),
    ])
    expect(container.textContent).toContain('Previous catalog')
    expect(
      container.querySelector<HTMLButtonElement>(
        '[data-model-option="xai::grok-4"]',
      )?.disabled,
    ).toBe(false)
  })

  it('retains the real reason when retry fails without exposing the thrown message', async () => {
    mocks.refresh.mockRejectedValueOnce(
      new Error('sk-PRIVATE https://private.invalid'),
    )
    await render([xai()])
    const retry = [...container.querySelectorAll('button')].find(
      (button) => button.textContent === 'Check again',
    )
    if (!retry) throw new Error('Missing retry')
    await act(async () => retry.click())
    expect(container.textContent).toContain('Credits or spending limit')
    expect(container.textContent).toContain('The last diagnostic has been kept.')
    expect(container.textContent).not.toContain('PRIVATE')
    expect(retry.disabled).toBe(false)
  })

  it('keeps feedback under a provider search and does not invent errors for speech-only providers', async () => {
    await render([
      xai(),
      {
        id: 'speech',
        display_name: 'Speech',
        available: true,
        configured: true,
        supports_model_listing: true,
      },
    ])
    expect(container.querySelector('[data-provider-group="speech"]')).toBeNull()
    const search = container.querySelector<HTMLInputElement>('input')
    if (!search) throw new Error('Missing search')
    await act(async () => {
      const setter = Object.getOwnPropertyDescriptor(
        HTMLInputElement.prototype,
        'value',
      )?.set
      setter?.call(search, 'xai')
      search.dispatchEvent(new Event('input', { bubbles: true }))
    })
    expect(container.textContent).toContain('Credits or spending limit')
    expect(container.querySelector('[data-provider-group="openai"]')).toBeNull()
  })

  it('drops unknown technical codes and upstream strings at the UI boundary', () => {
    const parsed = parseProviderList([
      {
        ...xai(),
        discovery: {
          ...billing,
          code: 'sk-PRIVATE https://private.invalid',
          message: 'team-PRIVATE',
        },
      },
    ])
    expect(JSON.stringify(parsed)).not.toContain('PRIVATE')
    expect(parsed[0].discovery?.code).toBeUndefined()
  })
})
