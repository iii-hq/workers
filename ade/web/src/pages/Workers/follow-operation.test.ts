import { describe, expect, it, vi } from 'vitest'
import type { Accepted, OperationSnapshot } from './compose-api'
import {
  followOperation,
  newOperationId,
  type ProgressEvent,
} from './follow-operation'

/** A bus that records bindings and lets a test deliver progress events. */
function bus() {
  const log: string[] = []
  const handlers = new Map<string, (payload: unknown) => void>()
  const bindings = new Map<
    number,
    { function_id: string; config: Record<string, unknown> }
  >()
  let next = 0
  const iii = {
    browserId: 'console-test',
    on: vi.fn((id: string, handler: (payload: never) => void) => {
      log.push(`on ${id}`)
      handlers.set(id, handler as (payload: unknown) => void)
      return () => {
        log.push(`off ${id}`)
        handlers.delete(id)
      }
    }),
    registerTrigger: vi.fn(
      (input: {
        type: string
        function_id: string
        config: Record<string, unknown>
      }) => {
        log.push(`bind ${input.type} ${String(input.config.operation_id)}`)
        const key = ++next
        bindings.set(key, input)
        return () => {
          log.push(`unbind ${String(input.config.operation_id)}`)
          bindings.delete(key)
        }
      },
    ),
  }
  const emit = (event: Partial<ProgressEvent> & { operation_id: string }) => {
    for (const { function_id, config } of bindings.values()) {
      if (config.operation_id !== event.operation_id) continue
      const handler = handlers.get(
        function_id.slice(0, -'::console-test'.length),
      )
      handler?.({
        sequence: 1,
        phase: 'starting',
        detail: '',
        terminal: false,
        ...event,
      })
    }
  }
  return { iii, log, emit, bindings, handlers }
}

const snapshot = (
  operation_id: string,
  patch: Partial<OperationSnapshot> = {},
): OperationSnapshot => ({
  operation_id,
  status: 'running',
  phase: 'resolving',
  completed: 0,
  total: 2,
  last_sequence: 1,
  last_event: { detail: 'resolving dependency trees', phase: 'resolving' },
  ...patch,
})

const accepted = (operation_id: string): Accepted => ({
  operation_id,
  requested: 1,
  status: 'accepted',
})

const flush = () => new Promise((resolve) => setTimeout(resolve, 0))

describe('followOperation', () => {
  it('chooses a compose operation id', () => {
    expect(newOperationId()).toMatch(/^compose:[0-9a-f-]{36}$/)
  })

  it('binds the progress before the mutation is submitted, under the id it passes', async () => {
    const { iii, log, emit } = bus()
    const start = vi.fn(async (id: string) => {
      log.push(`start ${id}`)
      return accepted(id)
    })
    const read = vi.fn(async (id: string) => snapshot(id))
    const done = followOperation({ iii, start, read, operationId: 'compose:a' })
    await flush()
    expect(log.slice(0, 3)).toEqual([
      expect.stringMatching(/^on iii::console::workers::operation::\d+$/),
      'bind compose-operation compose:a',
      'start compose:a',
    ])
    expect(read).toHaveBeenCalledTimes(1)
    read.mockResolvedValueOnce(
      snapshot('compose:a', { status: 'succeeded', phase: 'complete' }),
    )
    emit({
      operation_id: 'compose:a',
      sequence: 9,
      phase: 'complete',
      detail: 'done',
      terminal: true,
    })
    await expect(done).resolves.toMatchObject({ status: 'succeeded' })
    expect(read).toHaveBeenCalledTimes(2)
    expect(log.slice(-2)).toEqual([
      'unbind compose:a',
      expect.stringMatching(/^off /),
    ])
  })

  it('settles from the catch-up read when the operation ended before the binding landed', async () => {
    const { iii, bindings, handlers } = bus()
    const read = vi.fn(async (id: string) =>
      snapshot(id, {
        status: 'failed',
        last_event: { detail: 'no build', phase: 'complete' },
      }),
    )
    const outcome = await followOperation({
      iii,
      start: async (id) => accepted(id),
      read,
      operationId: 'compose:b',
    })
    expect(outcome).toMatchObject({
      status: 'failed',
      last_event: { detail: 'no build' },
    })
    expect(read).toHaveBeenCalledTimes(1)
    expect(bindings.size).toBe(0)
    expect(handlers.size).toBe(0)
  })

  it('reports each progress event while it runs, and ignores stale or foreign ones', async () => {
    const { iii, emit } = bus()
    const seen: string[] = []
    const read = vi.fn(async (id: string) => snapshot(id))
    const done = followOperation({
      iii,
      start: async (id) => accepted(id),
      read,
      operationId: 'compose:c',
      onProgress: (progress) =>
        seen.push(progress.last_event?.detail ?? progress.phase),
    })
    await flush()
    emit({
      operation_id: 'compose:c',
      sequence: 2,
      phase: 'starting',
      detail: 'starting worker',
      container: 'todo',
    })
    emit({
      operation_id: 'compose:c',
      sequence: 2,
      phase: 'starting',
      detail: 'duplicate',
    })
    emit({ operation_id: 'compose:other', sequence: 3, detail: 'not mine' })
    expect(seen).toEqual([
      'accepted',
      'resolving dependency trees',
      'starting worker',
    ])
    read.mockResolvedValueOnce(snapshot('compose:c', { status: 'cancelled' }))
    emit({
      operation_id: 'compose:c',
      sequence: 4,
      phase: 'complete',
      detail: 'operation cancelled',
      terminal: true,
    })
    await expect(done).resolves.toMatchObject({ status: 'cancelled' })
  })

  it('follows the id the daemon ran it under when it differs', async () => {
    const { iii, log, emit } = bus()
    const read = vi.fn(async (id: string) => snapshot(id))
    const done = followOperation({
      iii,
      start: async () => accepted('compose:daemon'),
      read,
      operationId: 'compose:mine',
    })
    await flush()
    expect(log).toContain('bind compose-operation compose:daemon')
    expect(log).toContain('unbind compose:mine')
    read.mockResolvedValueOnce(
      snapshot('compose:daemon', { status: 'succeeded' }),
    )
    emit({ operation_id: 'compose:daemon', sequence: 5, terminal: true })
    await expect(done).resolves.toMatchObject({
      operation_id: 'compose:daemon',
      status: 'succeeded',
    })
  })

  it('unbinds and settles with null when the page goes away', async () => {
    const { iii, bindings, handlers } = bus()
    const controller = new AbortController()
    const done = followOperation({
      iii,
      start: async (id) => accepted(id),
      read: async (id) => snapshot(id),
      operationId: 'compose:d',
      signal: controller.signal,
    })
    await flush()
    expect(bindings.size).toBe(1)
    controller.abort()
    await expect(done).resolves.toBeNull()
    expect(bindings.size).toBe(0)
    expect(handlers.size).toBe(0)
  })

  it('unbinds and rejects when the submission fails', async () => {
    const { iii, bindings, handlers } = bus()
    await expect(
      followOperation({
        iii,
        start: async () => {
          throw new Error('WRONG_DAEMON')
        },
        read: async (id) => snapshot(id),
      }),
    ).rejects.toThrow('WRONG_DAEMON')
    expect(bindings.size).toBe(0)
    expect(handlers.size).toBe(0)
  })
})
