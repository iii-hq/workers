// @vitest-environment jsdom
import { act } from 'react'
import { createRoot } from 'react-dom/client'
import { describe, expect, it, vi } from 'vitest'
import { AddProviderPanel } from './AddProviderPanel'

;(
  globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }
).IS_REACT_ACT_ENVIRONMENT = true

const { fetchRegistry } = vi.hoisted(() => ({
  fetchRegistry: vi.fn<() => Promise<unknown>>(),
}))
vi.mock('@/lib/workers-registry', async (importOriginal) => ({
  ...(await importOriginal<typeof import('@/lib/workers-registry')>()),
  fetchRegistryProviders: fetchRegistry,
}))

describe('AddProviderPanel search results', () => {
  it('says the registry is unreachable, never that nothing matches', async () => {
    fetchRegistry.mockRejectedValueOnce(new Error('offline'))
    fetchRegistry.mockResolvedValueOnce([
      {
        name: 'provider-deepseek',
        description: 'DeepSeek provider worker',
        version: '0.2.1',
        tags: ['provider'],
      },
    ])
    const counts: Array<number | null> = []
    const container = document.body.appendChild(document.createElement('div'))
    const root = createRoot(container)
    try {
      await act(async () =>
        root.render(
          <AddProviderPanel
            installedWorkerNames={[]}
            providers={[]}
            query="deep"
            onMatchCount={(count) => counts.push(count)}
          />,
        ),
      )
      expect(container.querySelector('[role="alert"]')?.textContent).toContain(
        'registry is unreachable',
      )
      // Unknown, not zero: the dialog keeps its "No match" note back.
      expect(counts).not.toContain(0)
      const retry = [...container.querySelectorAll('button')].find(
        (button) => button.textContent === 'Retry',
      )
      await act(async () => retry?.click())
      expect(
        container.querySelector('[data-registry-provider="provider-deepseek"]'),
      ).not.toBeNull()
      expect(counts.at(-1)).toBe(1)
    } finally {
      act(() => root.unmount())
      container.remove()
    }
  })
})
