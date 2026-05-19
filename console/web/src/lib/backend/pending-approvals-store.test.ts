import { describe, expect, it } from 'vitest'
import {
  diffPending,
  type PendingApproval,
} from './pending-approvals-store'

describe('diffPending', () => {
  it('returns all entries as added when previous list is empty', () => {
    const next: PendingApproval[] = [
      { function_call_id: 'fc-1', function_id: 'shell::fs::write', args: { path: '/tmp/x' } },
    ]
    expect(diffPending([], next)).toEqual({
      added: next,
      removed: [],
    })
  })

  it('returns all entries as removed when next list is empty', () => {
    const prev: PendingApproval[] = [
      { function_call_id: 'fc-1', function_id: 'shell::fs::write', args: { path: '/tmp/x' } },
    ]
    expect(diffPending(prev, [])).toEqual({
      added: [],
      removed: prev,
    })
  })

  it('detects a single addition', () => {
    const prev: PendingApproval[] = [
      { function_call_id: 'fc-1', function_id: 'shell::fs::write', args: {} },
    ]
    const next: PendingApproval[] = [
      ...prev,
      { function_call_id: 'fc-2', function_id: 'shell::shell', args: {} },
    ]
    expect(diffPending(prev, next)).toEqual({
      added: [next[1]],
      removed: [],
    })
  })

  it('detects a single removal', () => {
    const prev: PendingApproval[] = [
      { function_call_id: 'fc-1', function_id: 'shell::fs::write', args: {} },
      { function_call_id: 'fc-2', function_id: 'shell::shell', args: {} },
    ]
    const next: PendingApproval[] = [prev[0]!]
    expect(diffPending(prev, next)).toEqual({
      added: [],
      removed: [prev[1]],
    })
  })

  it('is keyed by function_call_id (treats objects with same id as same entry)', () => {
    const prev: PendingApproval[] = [
      { function_call_id: 'fc-1', function_id: 'shell::shell', args: { command: 'ls' } },
    ]
    const next: PendingApproval[] = [
      { function_call_id: 'fc-1', function_id: 'shell::shell', args: { command: 'rm' } },
    ]
    expect(diffPending(prev, next)).toEqual({ added: [], removed: [] })
  })
})
