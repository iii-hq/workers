import { describe, expect, it } from 'vitest'
import { bufferTerminalFrame, mergeTerminalFrames } from '../terminal-stream'

describe('mergeTerminalFrames', () => {
  it('deduplicates overlap and orders replay with queued live frames', () => {
    expect(
      mergeTerminalFrames(
        [
          { sequence: 4, data: 'four' },
          { sequence: 5, data: 'five' },
        ],
        [
          { sequence: 5, data: 'five' },
          { sequence: 6, data: 'six' },
        ],
        3,
      ),
    ).toEqual([
      { sequence: 4, data: 'four' },
      { sequence: 5, data: 'five' },
      { sequence: 6, data: 'six' },
    ])
  })

  it('discards frames at or below afterSequence', () => {
    expect(
      mergeTerminalFrames(
        [
          { sequence: 2, data: 'two' },
          { sequence: 3, data: 'three' },
        ],
        [{ sequence: 3, data: 'three' }],
        3,
      ),
    ).toEqual([])
  })

  it('rejects conflicting data for one sequence', () => {
    expect(() =>
      mergeTerminalFrames(
        [{ sequence: 4, data: 'four-a' }],
        [{ sequence: 4, data: 'four-b' }],
        3,
      ),
    ).toThrow(/conflicting terminal frame data for sequence 4/)
  })

  it('rejects invalid afterSequence values', () => {
    expect(() =>
      mergeTerminalFrames([{ sequence: 4, data: 'four' }], [], -1),
    ).toThrow(/invalid afterSequence/)
    expect(() =>
      mergeTerminalFrames([{ sequence: 4, data: 'four' }], [], 1.5),
    ).toThrow(/invalid afterSequence/)
    expect(() =>
      mergeTerminalFrames([{ sequence: 4, data: 'four' }], [], Number.NaN),
    ).toThrow(/invalid afterSequence/)
  })

  it('rejects invalid frame sequences in replay and pending', () => {
    expect(() =>
      mergeTerminalFrames([{ sequence: -1, data: 'bad' }], [], 0),
    ).toThrow(/invalid frame sequence/)
    expect(() =>
      mergeTerminalFrames([], [{ sequence: 1.5, data: 'bad' }], 0),
    ).toThrow(/invalid frame sequence/)
    expect(() =>
      mergeTerminalFrames(
        [{ sequence: Number.MAX_SAFE_INTEGER + 1, data: 'bad' }],
        [],
        0,
      ),
    ).toThrow(/invalid frame sequence/)
  })
})

describe('bufferTerminalFrame', () => {
  it('holds out-of-order frames until gaps are filled', () => {
    const pending = new Map()

    expect(
      bufferTerminalFrame(pending, { sequence: 12, data: 'twelve' }, 10),
    ).toEqual([])
    expect(
      bufferTerminalFrame(pending, { sequence: 11, data: 'eleven' }, 10),
    ).toEqual([
      { sequence: 11, data: 'eleven' },
      { sequence: 12, data: 'twelve' },
    ])
    expect(pending.size).toBe(0)
  })
})
