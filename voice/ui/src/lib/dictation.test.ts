import { describe, expect, it } from 'vitest'
import { initialDictationReduceState, reduceTranscriptEvent } from './dictation'
import type { TranscriptEvent } from './types'

function event(overrides: Partial<TranscriptEvent>): TranscriptEvent {
  return {
    session_id: 's1',
    seq: 0,
    kind: 'partial',
    text: '',
    segment: 0,
    timestamp_ms: 0,
    ...overrides,
  }
}

describe('reduceTranscriptEvent', () => {
  it('replaces the partial text on a partial event and marks listening', () => {
    const state = reduceTranscriptEvent(initialDictationReduceState, event({ seq: 1, kind: 'partial', text: 'hel' }))
    expect(state.partial).toBe('hel')
    expect(state.status).toBe('listening')
    expect(state.committed).toEqual([])
  })

  it('commits a final segment and clears the partial', () => {
    let state = reduceTranscriptEvent(initialDictationReduceState, event({ seq: 1, kind: 'partial', text: 'hello' }))
    state = reduceTranscriptEvent(state, event({ seq: 2, kind: 'final', text: 'hello there' }))
    expect(state.committed).toEqual(['hello there'])
    expect(state.partial).toBe('')
    expect(state.status).toBe('listening')
  })

  it('accumulates multiple final segments in order', () => {
    let state = initialDictationReduceState
    state = reduceTranscriptEvent(state, event({ seq: 1, kind: 'final', text: 'first' }))
    state = reduceTranscriptEvent(state, event({ seq: 2, kind: 'partial', text: 'sec' }))
    state = reduceTranscriptEvent(state, event({ seq: 3, kind: 'final', text: 'second' }))
    expect(state.committed).toEqual(['first', 'second'])
    expect(state.partial).toBe('')
  })

  it('ends the session on a closed event and clears the partial', () => {
    let state = reduceTranscriptEvent(initialDictationReduceState, event({ seq: 1, kind: 'partial', text: 'hi' }))
    state = reduceTranscriptEvent(state, event({ seq: 2, kind: 'closed', text: '' }))
    expect(state.status).toBe('idle')
    expect(state.partial).toBe('')
  })

  it('surfaces an error event with its reason', () => {
    const state = reduceTranscriptEvent(
      initialDictationReduceState,
      event({ seq: 1, kind: 'error', reason: 'mic lost' }),
    )
    expect(state.status).toBe('error')
    expect(state.error).toBe('mic lost')
  })

  it('falls back to a generic message when an error event has no reason', () => {
    const state = reduceTranscriptEvent(initialDictationReduceState, event({ seq: 1, kind: 'error' }))
    expect(state.error).toBe('dictation error')
  })

  it('ignores an out-of-order or duplicate seq', () => {
    let state = reduceTranscriptEvent(initialDictationReduceState, event({ seq: 5, kind: 'final', text: 'later' }))
    const afterFirst = state
    state = reduceTranscriptEvent(state, event({ seq: 3, kind: 'final', text: 'earlier, arrived late' }))
    expect(state).toEqual(afterFirst)
    state = reduceTranscriptEvent(state, event({ seq: 5, kind: 'final', text: 'duplicate of the same seq' }))
    expect(state).toEqual(afterFirst)
  })
})
