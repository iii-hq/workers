import type { Host } from '@iii-dev/console-ui'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { TURNS_CHANGED_DEBOUNCE_MS, useTurnsChanged } from '../turn'
import { mount } from './bare-hooks'

vi.mock('react', async (original) => ({
  ...(await original<typeof import('react')>()),
  ...(await import('./bare-hooks')).hooks,
}))

function engine() {
  const handlers = new Map<string, (payload: unknown) => void>()
  const triggers: Array<{ type: string; function_id: string; config: Record<string, unknown> }> = []
  const host = {
    iii: {
      browserId: 'tab',
      on: (functionId: string, handler: (payload: unknown) => void) => {
        handlers.set(functionId, handler)
        return () => handlers.delete(functionId)
      },
      registerTrigger: (input: (typeof triggers)[number]) => {
        triggers.push(input)
        return () => triggers.splice(triggers.indexOf(input), 1)
      },
    },
  } as unknown as Host
  const fire = (payload: unknown) => {
    for (const t of triggers) handlers.get(t.function_id.replace(/::tab$/, ''))?.(payload)
  }
  return { host, handlers, triggers, fire }
}

describe('useTurnsChanged', () => {
  afterEach(() => {
    vi.useRealTimers()
  })

  it('reads the chat’s turns once per burst the worker reports, and on no timer', async () => {
    vi.useFakeTimers()
    const { host, handlers, triggers, fire } = engine()
    let reads = 0
    const pane = mount(
      () =>
        useTurnsChanged(host, 'chat-1', 'pane-a', () => {
          reads += 1
        }),
      undefined,
    )
    expect(triggers).toEqual([
      {
        type: 'shell::turns::changed',
        function_id: 'iii::shell-ui::turns-changed::pane-a::tab',
        config: { session_id: 'chat-1' },
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
    expect(triggers).toEqual([])
    expect(handlers.size).toBe(0)
  })

  it('binds nothing without a chat', () => {
    const { host, triggers } = engine()
    const pane = mount(() => useTurnsChanged(host, null, 'pane-a', () => {}), undefined)
    expect(triggers).toEqual([])
    pane.unmount()
  })
})
