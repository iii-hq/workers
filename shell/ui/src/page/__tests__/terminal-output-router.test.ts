import { describe, expect, it } from 'vitest'
import {
  createTerminalOutputRouter,
  type PtyOutputEvent,
} from '../terminal-output-router'

function event(
  sessionId: string,
  sequence: number,
  data = 'eA==',
): PtyOutputEvent {
  return {
    session_id: sessionId,
    sequence,
    data,
    eof: false,
    exit_code: null,
    signal: null,
    error: null,
  }
}

describe('TerminalOutputRouter', () => {
  it('routes interleaved events by session id', () => {
    let emit: ((event: PtyOutputEvent) => void) | undefined
    const host = {
      iii: {
        browserId: 'console-test',
        on: (_id: string, listener: (event: PtyOutputEvent) => void) => {
          emit = listener
          return () => {
            emit = undefined
          }
        },
      },
    } as never
    const router = createTerminalOutputRouter(host)
    const first: number[] = []
    const second: number[] = []
    router.subscribe('session-1', (output) => first.push(output.sequence))
    router.subscribe('session-2', (output) => second.push(output.sequence))

    emit?.(event('session-2', 1))
    emit?.(event('session-1', 1))
    emit?.(event('session-2', 2))

    expect(first).toEqual([1])
    expect(second).toEqual([1, 2])
    router.dispose()
  })

  it('bounds unsubscribed output queues to 2 MiB of raw data', () => {
    let emit: ((event: PtyOutputEvent) => void) | undefined
    const host = {
      iii: {
        browserId: 'console-test',
        on: (_id: string, listener: (event: PtyOutputEvent) => void) => {
          emit = listener
          return () => undefined
        },
      },
    } as never
    const router = createTerminalOutputRouter(host)
    const oneMiB = btoa('x'.repeat(1024 * 1024))

    emit?.(event('session-1', 1, oneMiB))
    emit?.(event('session-1', 2, oneMiB))
    emit?.(event('session-1', 3, oneMiB))

    expect(router.drain('session-1').map((output) => output.sequence)).toEqual([
      2, 3,
    ])
    expect(router.drain('session-1')).toEqual([])
    router.dispose()
  })
})
