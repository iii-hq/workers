import { describe, expect, it } from 'vitest'
import {
  type PendingApprovalCandidate,
  nextApprovalsToAutoResolve,
} from './auto-accept'

function pending(
  ...ids: Array<[functionCallId: string, sessionId: string]>
): PendingApprovalCandidate[] {
  return ids.map(([functionCallId, sessionId]) => ({ functionCallId, sessionId }))
}

describe('nextApprovalsToAutoResolve', () => {
  it('returns the pending entries minus the ones already resolved', () => {
    const input = pending(['fc-1', 's-1'], ['fc-2', 's-1'], ['fc-3', 's-1'])
    const seen = new Set(['fc-2'])
    expect(nextApprovalsToAutoResolve(input, seen)).toEqual([
      { functionCallId: 'fc-1', sessionId: 's-1' },
      { functionCallId: 'fc-3', sessionId: 's-1' },
    ])
  })

  it('returns an empty array when every pending entry is already resolved', () => {
    const input = pending(['fc-1', 's-1'], ['fc-2', 's-1'])
    const seen = new Set(['fc-1', 'fc-2'])
    expect(nextApprovalsToAutoResolve(input, seen)).toEqual([])
  })

  it('returns an empty array on empty input (the no-op happy path)', () => {
    expect(nextApprovalsToAutoResolve([], new Set())).toEqual([])
    expect(nextApprovalsToAutoResolve([], new Set(['fc-99']))).toEqual([])
  })

  it('preserves input order when filtering', () => {
    const input = pending(['fc-a', 's'], ['fc-b', 's'], ['fc-c', 's'])
    const seen = new Set(['fc-b'])
    expect(nextApprovalsToAutoResolve(input, seen).map((p) => p.functionCallId)).toEqual([
      'fc-a',
      'fc-c',
    ])
  })

  it('does not mutate the input arrays or set', () => {
    const input = pending(['fc-1', 's-1'], ['fc-2', 's-1'])
    const inputBefore = JSON.stringify(input)
    const seen = new Set(['fc-1'])
    const seenBefore = [...seen]
    nextApprovalsToAutoResolve(input, seen)
    expect(JSON.stringify(input)).toBe(inputBefore)
    expect([...seen]).toEqual(seenBefore)
  })
})
