import { describe, expect, it } from 'vitest'
import { createOutputRouter, OUTPUT_HANDLER } from '../output-router'
import type { TerminalOutputEvent } from '../types'

function fakeIii() {
  let handler: ((payload: TerminalOutputEvent) => void) | null = null
  let registered = ''
  let offs = 0
  return {
    browserId: 'console-abc',
    on<P>(functionId: string, fn: (payload: P) => void) {
      registered = functionId
      handler = fn as unknown as (payload: TerminalOutputEvent) => void
      return () => {
        offs += 1
      }
    },
    emit(event: TerminalOutputEvent) {
      handler?.(event)
    },
    registered: () => registered,
    offs: () => offs,
  }
}

const event = (terminal: string, sequence: number): TerminalOutputEvent => ({
  terminal_id: terminal,
  sequence,
  data: 'eA==',
  eof: false,
  exit_code: null,
  error: null,
})

describe('createOutputRouter', () => {
  it('registers one tab-scoped handler and names it for open', () => {
    const iii = fakeIii()
    const router = createOutputRouter(iii)
    expect(iii.registered()).toBe(OUTPUT_HANDLER)
    expect(router.outputFunctionId).toBe('iii::tmux-ui::pty-output::console-abc')
  })

  it('queues output that beats the subscription and hands it over on drain', () => {
    const iii = fakeIii()
    const router = createOutputRouter(iii)
    iii.emit(event('t1', 1))
    iii.emit(event('t1', 2))
    const seen: number[] = []
    router.subscribe('t1', (e) => seen.push(e.sequence ?? -1))
    iii.emit(event('t1', 3))
    expect(seen).toEqual([3])
    expect(router.drain('t1').map((e) => e.sequence)).toEqual([1, 2])
    expect(router.drain('t1')).toEqual([])
  })

  it('routes by terminal and forgets discarded queues', () => {
    const iii = fakeIii()
    const router = createOutputRouter(iii)
    const seen: string[] = []
    router.subscribe('a', (e) => seen.push(e.terminal_id))
    iii.emit(event('a', 1))
    iii.emit(event('b', 1))
    router.discard('b')
    expect(seen).toEqual(['a'])
    expect(router.drain('b')).toEqual([])
  })

  it('stops listening on dispose', () => {
    const iii = fakeIii()
    const router = createOutputRouter(iii)
    router.dispose()
    router.dispose()
    expect(iii.offs()).toBe(1)
  })
})
