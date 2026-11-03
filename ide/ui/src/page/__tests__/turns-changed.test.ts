import type { Host, ObserveSource } from '@iii-dev/console-ui'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { TURNS_CHANGED_DEBOUNCE_MS, useTurnsChanged } from '../turn'
import { mount } from './bare-hooks'

vi.mock('react', async (original) => ({
  ...(await original<typeof import('react')>()),
  ...(await import('./bare-hooks')).hooks,
}))

function engine() {
  const handlers = new Map<string, (payload: unknown) => void>()
  const observers: Array<{ source: ObserveSource; target: string }> = []
  const host = {
    iii: {
      browserId: 'tab',
      on: (functionId: string, handler: (payload: unknown) => void) => {
        handlers.set(functionId, handler)
        return () => handlers.delete(functionId)
      },
      observe: (source: ObserveSource, target: string) => {
        const observer = { source, target }
        observers.push(observer)
        return () => observers.splice(observers.indexOf(observer), 1)
      },
    },
  } as unknown as Host
  const fire = (payload: unknown) => {
    for (const o of observers) handlers.get(o.target.replace(/::tab$/, ''))?.(payload)
  }
  return { host, handlers, observers, fire }
}

describe('useTurnsChanged', () => {
  afterEach(() => {
    vi.useRealTimers()
  })

  it('reads the chat’s turns once per burst the worker reports, and on no timer', async () => {
    vi.useFakeTimers()
    const { host, handlers, observers, fire } = engine()
    let reads = 0
    const pane = mount(
      () =>
        useTurnsChanged(host, 'chat-1', 'pane-a', () => {
          reads += 1
        }),
      undefined,
    )
    expect(observers).toEqual([
      {
        source: {
          function_id: 'shell::turns::on-changed',
          filters: { 'input.session_id': { $eq: 'chat-1' } },
        },
        target: 'iii::shell-ui::turns-changed::pane-a::tab',
      },
    ])
    await vi.advanceTimersByTimeAsync(60_000)
    expect(reads).toBe(0)

    fire({ session_id: 'chat-2' })
    fire({ session_id: 'chat-1' })
    fire({ session_id: 'chat-1' })
    await vi.advanceTimersByTimeAsync(TURNS_CHANGED_DEBOUNCE_MS)
    expect(reads).toBe(1)

    pane.unmount()
    expect(observers).toEqual([])
    expect(handlers.size).toBe(0)
  })

  it('observes nothing without a chat', () => {
    const { host, observers } = engine()
    const pane = mount(() => useTurnsChanged(host, null, 'pane-a', () => {}), undefined)
    expect(observers).toEqual([])
    pane.unmount()
  })
})
