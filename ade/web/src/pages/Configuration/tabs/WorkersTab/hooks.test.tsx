// @vitest-environment jsdom

import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { act } from 'react'
import { createRoot } from 'react-dom/client'
import { describe, expect, it, vi } from 'vitest'

const client = vi.hoisted(() => {
  const handlers = new Map<string, (payload: unknown) => void>()
  return {
    handlers,
    browserId: 'tab',
    on: (id: string, handler: (payload: unknown) => void) => {
      handlers.set(id, handler)
      return () => handlers.delete(id)
    },
    observe: vi.fn(
      (
        _source: { function_id: string; filters?: unknown },
        _target: string,
      ) => vi.fn(),
    ),
  }
})
vi.mock('@/lib/iii-client', () => ({ getIiiClient: async () => client }))
vi.mock('@/lib/backend', () => ({ getDefaultBackend: () => ({ id: 'real' }) }))

import { configurationKeys, useWorkerRegistryReactivity } from './hooks'

;(
  globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }
).IS_REACT_ACT_ENVIRONMENT = true

describe('useWorkerRegistryReactivity', () => {
  it('re-pulls the list when a configuration registers after the worker connected', async () => {
    const qc = new QueryClient()
    const invalidate = vi.spyOn(qc, 'invalidateQueries')
    function Probe() {
      useWorkerRegistryReactivity()
      return null
    }
    const root = createRoot(document.createElement('div'))
    await act(async () => {
      root.render(
        <QueryClientProvider client={qc}>
          <Probe />
        </QueryClientProvider>,
      )
    })

    const call = client.observe.mock.calls.find(
      ([source]) => source.function_id === 'configuration::on-updated',
    )
    expect(call?.[0].filters).toEqual({
      'input.event_type': {
        $in: ['configuration:registered', 'configuration:updated'],
      },
    })
    client.handlers.get(call?.[1].replace(/::tab$/, '') ?? '')?.({
      event_type: 'configuration:registered',
      id: 'ide',
    })
    expect(invalidate).toHaveBeenCalledWith({
      queryKey: configurationKeys.list(),
    })

    act(() => root.unmount())
    expect(client.handlers.size).toBe(0)
  })
})
