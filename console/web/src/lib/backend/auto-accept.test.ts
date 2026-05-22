import { describe, expect, it } from 'vitest'
import type {
  FunctionCallMessage,
  Message,
  UserMessage,
} from '@/types/chat'
import {
  nextApprovalsToAutoResolve,
  selectAutoAcceptCandidates,
  type PendingApprovalCandidate,
} from './auto-accept'

function pending(
  ...ids: Array<[functionCallId: string, sessionId: string]>
): PendingApprovalCandidate[] {
  return ids.map(([functionCallId, sessionId]) => ({
    functionCallId,
    sessionId,
    functionId: 'directory::skills::list',
  }))
}

function userMsg(id: string, content = 'hi'): UserMessage {
  return { id, role: 'user', content, createdAt: 0 }
}

function pendingFcall(
  id: string,
  functionCallId: string,
  functionId = 'directory::skills::list',
  sessionId = 'sess-1',
): FunctionCallMessage {
  return {
    id,
    role: 'function-call',
    functionId,
    input: {},
    pendingApproval: true,
    running: false,
    functionCallId,
    sessionId,
    createdAt: 0,
  }
}

describe('nextApprovalsToAutoResolve', () => {
  it('returns the pending entries minus the ones already resolved', () => {
    const input = pending(['fc-1', 's-1'], ['fc-2', 's-1'], ['fc-3', 's-1'])
    const seen = new Set(['fc-2'])
    expect(nextApprovalsToAutoResolve(input, seen).map((p) => p.functionCallId)).toEqual([
      'fc-1',
      'fc-3',
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

describe('selectAutoAcceptCandidates', () => {
  it('returns null when the message list is empty (hot-path early exit)', () => {
    expect(selectAutoAcceptCandidates([])).toBeNull()
  })

  it('returns null when the tail message is not a function-call', () => {
    const messages: Message[] = [pendingFcall('m-1', 'fc-1'), userMsg('m-2')]
    expect(selectAutoAcceptCandidates(messages)).toBeNull()
  })

  it('returns candidates when the tail IS a function-call', () => {
    const messages: Message[] = [pendingFcall('m-1', 'fc-1')]
    const out = selectAutoAcceptCandidates(messages)
    expect(out).not.toBeNull()
    expect(out!.candidates).toHaveLength(1)
    expect(out!.candidates[0]).toEqual({
      functionCallId: 'fc-1',
      sessionId: 'sess-1',
      functionId: 'directory::skills::list',
    })
  })

  it('separates policy-denied entries from acceptable candidates', () => {
    const messages: Message[] = [
      pendingFcall('m-1', 'fc-1', 'directory::skills::list'),
      pendingFcall('m-2', 'fc-2', 'fs::write'),
      pendingFcall('m-3', 'fc-3', 'agent::trigger'),
      pendingFcall('m-4', 'fc-4', 'fs::ls'),
    ]
    const out = selectAutoAcceptCandidates(messages)!
    expect(out.candidates.map((c) => c.functionCallId)).toEqual(['fc-1', 'fc-4'])
    expect(out.deniedByPolicy.map((c) => c.functionCallId)).toEqual(['fc-2', 'fc-3'])
  })

  it('skips function-call messages that lack functionCallId or sessionId', () => {
    const incomplete: FunctionCallMessage = {
      id: 'm-x',
      role: 'function-call',
      functionId: 'fs::ls',
      input: {},
      pendingApproval: true,
      createdAt: 0,
    }
    const messages: Message[] = [incomplete, pendingFcall('m-1', 'fc-1')]
    const out = selectAutoAcceptCandidates(messages)!
    expect(out.candidates.map((c) => c.functionCallId)).toEqual(['fc-1'])
  })

  it('skips function-call messages without pendingApproval=true', () => {
    const settled: FunctionCallMessage = {
      ...pendingFcall('m-1', 'fc-1'),
      pendingApproval: false,
      running: false,
    }
    const messages: Message[] = [settled, pendingFcall('m-2', 'fc-2')]
    const out = selectAutoAcceptCandidates(messages)!
    expect(out.candidates.map((c) => c.functionCallId)).toEqual(['fc-2'])
  })
})
