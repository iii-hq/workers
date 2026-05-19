import { describe, expect, it } from 'vitest'
import { pendingApprovalsFromTurnState } from './turn-state-mirror'

describe('pendingApprovalsFromTurnState', () => {
  it('returns [] when the record is null', () => {
    expect(pendingApprovalsFromTurnState(null)).toEqual([])
  })

  it('returns [] when state is not function_awaiting_approval', () => {
    expect(
      pendingApprovalsFromTurnState({
        state: 'function_execute',
        awaiting_approval: [
          { function_call_id: 'fc-1', function_id: 'shell::shell', args: {} },
        ],
      }),
    ).toEqual([])
  })

  it('returns [] when awaiting_approval is missing or empty', () => {
    expect(
      pendingApprovalsFromTurnState({
        state: 'function_awaiting_approval',
        awaiting_approval: [],
      }),
    ).toEqual([])
    expect(
      pendingApprovalsFromTurnState({
        state: 'function_awaiting_approval',
      }),
    ).toEqual([])
  })

  it('extracts pending entries when state is function_awaiting_approval', () => {
    const entries = [
      {
        function_call_id: 'fc-1',
        function_id: 'shell::fs::write',
        args: { path: '/tmp/x' },
      },
    ]
    expect(
      pendingApprovalsFromTurnState({
        state: 'function_awaiting_approval',
        awaiting_approval: entries,
      }),
    ).toEqual(entries)
  })

  it('filters malformed entries (missing function_call_id or function_id)', () => {
    expect(
      pendingApprovalsFromTurnState({
        state: 'function_awaiting_approval',
        awaiting_approval: [
          { function_call_id: 'fc-1', function_id: 'shell::shell', args: {} },
          { function_call_id: 'fc-2' }, // missing function_id
          { function_id: 'shell::shell', args: {} }, // missing function_call_id
          'not-an-object',
        ],
      }),
    ).toEqual([
      { function_call_id: 'fc-1', function_id: 'shell::shell', args: {} },
    ])
  })
})
