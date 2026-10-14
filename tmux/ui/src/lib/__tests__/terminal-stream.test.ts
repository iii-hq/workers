import { describe, expect, it } from 'vitest'
import { bufferTerminalFrame, createOrderedWriter } from '../terminal-stream'
import type { TerminalOutputEvent } from '../types'

function event(sequence: number | null, data: string | null, eof = false): TerminalOutputEvent {
  return { terminal_id: 't1', sequence, data, eof, exit_code: eof ? 0 : null, error: null }
}

function harness() {
  const written: string[] = []
  const ended: TerminalOutputEvent[] = []
  let stalls = 0
  const timers = new Map<number, () => void>()
  let nextTimer = 1
  const writer = createOrderedWriter({
    write: (data) => written.push(data),
    onEof: (e) => ended.push(e),
    onStall: () => {
      stalls += 1
    },
    setTimer: (callback) => {
      const id = nextTimer++
      timers.set(id, callback)
      return id
    },
    clearTimer: (handle) => {
      timers.delete(handle as number)
    },
  })
  const fireTimers = () => {
    const pending = [...timers.values()]
    timers.clear()
    for (const callback of pending) callback()
  }
  return { writer, written, ended, fireTimers, stalls: () => stalls, timers }
}

describe('bufferTerminalFrame', () => {
  it('holds a frame until the gap before it closes', () => {
    const pending = new Map()
    expect(bufferTerminalFrame(pending, { sequence: 2, data: 'b' }, 0)).toEqual([])
    expect(bufferTerminalFrame(pending, { sequence: 1, data: 'a' }, 0)).toEqual([
      { sequence: 1, data: 'a' },
      { sequence: 2, data: 'b' },
    ])
    expect(pending.size).toBe(0)
  })

  it('drops duplicates at or below the applied sequence', () => {
    expect(bufferTerminalFrame(new Map(), { sequence: 3, data: 'x' }, 3)).toEqual([])
  })

  it('refuses two different payloads for one sequence', () => {
    const pending = new Map([[5, { sequence: 5, data: 'one' }]])
    expect(() => bufferTerminalFrame(pending, { sequence: 5, data: 'two' }, 3)).toThrow(/conflicting/)
  })
})

describe('createOrderedWriter', () => {
  it('paints frames in sequence order whatever order they arrive in', () => {
    const { writer, written } = harness()
    writer.feed(event(3, 'c'))
    writer.feed(event(1, 'a'))
    expect(written).toEqual(['a'])
    writer.feed(event(2, 'b'))
    expect(written).toEqual(['a', 'b', 'c'])
    expect(writer.lastSequence()).toBe(3)
  })

  it('asks for a replay when a gap outlives the stall timer, and not after it closes', () => {
    const h = harness()
    h.writer.feed(event(2, 'b'))
    expect(h.timers.size).toBe(1)
    h.fireTimers()
    expect(h.stalls()).toBe(1)

    const closed = harness()
    closed.writer.feed(event(2, 'b'))
    closed.writer.feed(event(1, 'a'))
    expect(closed.timers.size).toBe(0)
    expect(closed.stalls()).toBe(0)
  })

  it('applies replayed frames into the gap', () => {
    const { writer, written } = harness()
    writer.feed(event(1, 'a'))
    writer.feed(event(4, 'd'))
    writer.feedFrames([
      { sequence: 2, data: 'b' },
      { sequence: 3, data: 'c' },
    ])
    expect(written).toEqual(['a', 'b', 'c', 'd'])
  })

  it('holds the sequence-less eof until the output before it is painted', () => {
    const { writer, written, ended } = harness()
    writer.feed(event(2, 'b'))
    writer.feed(event(null, null, true))
    expect(ended).toEqual([])
    writer.feed(event(1, 'a'))
    expect(written).toEqual(['a', 'b'])
    expect(ended).toHaveLength(1)
  })

  it('skips a truncated range and continues with what follows it', () => {
    const { writer, written } = harness()
    writer.feed(event(1, 'a'))
    writer.feed(event(9, 'i'))
    writer.skipTo(8)
    expect(written).toEqual(['a', 'i'])
    expect(writer.lastSequence()).toBe(9)
    expect(writer.pendingCount()).toBe(0)
  })

  it('starts over for a new terminal', () => {
    const { writer, written } = harness()
    writer.feed(event(1, 'a'))
    writer.feed(event(3, 'stale'))
    writer.reset()
    writer.feed(event(1, 'fresh'))
    expect(written).toEqual(['a', 'fresh'])
    expect(writer.pendingCount()).toBe(0)
  })
})
