// @vitest-environment jsdom

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
    registerTrigger: vi.fn((_input: { function_id: string }) => vi.fn()),
  }
})
vi.mock('@/lib/iii-client', () => ({ getIiiClient: async () => client }))

import { useWorkersChanged } from './use-workers-changed'

;(
  globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }
).IS_REACT_ACT_ENVIRONMENT = true

describe('useWorkersChanged', () => {
  it('runs onChange on every engine::workers-available event', async () => {
    const onChange = vi.fn()
    function Probe() {
      useWorkersChanged({
        enabled: true,
        fnId: 'console::test-watch',
        onChange,
      })
      return null
    }
    const root = createRoot(document.createElement('div'))
    await act(async () => {
      root.render(<Probe />)
    })

    expect(client.registerTrigger).toHaveBeenCalledOnce()
    const [input] = client.registerTrigger.mock.calls[0] as unknown as [
      { type: string; function_id: string; config: unknown },
    ]
    expect(input.type).toBe('engine::workers-available')
    expect(input.config).toEqual({})
    const handler = client.handlers.get(input.function_id.replace(/::tab$/, ''))
    handler?.({ event: 'worker_connected', worker_id: 'w-1' })
    handler?.({ event: 'worker_disconnected', worker_id: 'w-1' })
    expect(onChange).toHaveBeenCalledTimes(2)

    act(() => root.unmount())
    expect(client.handlers.size).toBe(0)
  })
})
