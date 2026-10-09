// @vitest-environment jsdom
import { act } from 'react'
import { createRoot } from 'react-dom/client'
import { expect, it, vi } from 'vitest'
import type { ProviderListEntry } from '@/lib/models-catalog'
import { useModelPickerSource } from './use-model-picker-source'

const mocks = vi.hoisted(() => ({ providers: vi.fn(), catalog: vi.fn() }))
vi.mock('@/lib/models-catalog', async (original) => ({
  ...(await original<typeof import('@/lib/models-catalog')>()),
  fetchProviderList: mocks.providers,
  fetchModelsCatalog: mocks.catalog,
  subscribeModelsChanged: async () => () => {},
  subscribeProviderChanges: async () => () => {},
}))
;(
  globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }
).IS_REACT_ACT_ENVIRONMENT = true

it('an older provider/catalog snapshot cannot overwrite a newer retry result', async () => {
  let first!: (providers: ProviderListEntry[]) => void
  let second!: (providers: ProviderListEntry[]) => void
  mocks.providers
    .mockImplementationOnce(
      () => new Promise((resolve) => { first = resolve }),
    )
    .mockImplementationOnce(
      () => new Promise((resolve) => { second = resolve }),
    )
  mocks.catalog.mockResolvedValue([])
  let source!: ReturnType<typeof useModelPickerSource>
  function Probe() {
    source = useModelPickerSource('real', true)
    return <output>{source.presentProviders[0]?.discovery?.outcome}</output>
  }
  const container = document.body.appendChild(document.createElement('div'))
  const root = createRoot(container)
  const provider = (outcome: 'billing' | 'success'): ProviderListEntry => ({
    id: 'xai',
    display_name: 'xAI',
    supports_model_listing: true,
    available: true,
    discovery: { outcome, stale: false, checked_at_ms: 1 },
  })
  try {
    await act(async () => root.render(<Probe />))
    let refresh!: Promise<void>
    await act(async () => { refresh = source.refresh() })
    await act(async () => { second([provider('success')]); await refresh })
    expect(container.textContent).toBe('success')
    await act(async () => first([provider('billing')]))
    expect(container.textContent).toBe('success')
  } finally {
    act(() => root.unmount())
    container.remove()
  }
})
