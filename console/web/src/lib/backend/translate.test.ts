import { describe, expect, it } from 'vitest'
import type {
  PendingApprovalRecord,
  PendingResolvedEvent,
  TurnCompletedEvent,
} from '@/types/iii-agent-event'
import {
  isTerminalSource,
  type TurnSourceEvent,
  translateTurnSource,
} from './translate'

function pendingRecord(
  over: Partial<PendingApprovalRecord> = {},
): PendingApprovalRecord {
  return {
    function_call_id: 'fc-1',
    function_id: 'shell::fs::write',
    session_id: 'sess-a',
    turn_id: 't-1',
    arguments_excerpt: { path: '/tmp/x' },
    pending_at: 0,
    expires_at: 1_000,
    status: 'pending',
    ...over,
  }
}

describe('translateTurnSource — approvals', () => {
  it('maps approval-created to a pending fcall-start carrying the redacted args', () => {
    const event: TurnSourceEvent = {
      kind: 'approval-created',
      record: pendingRecord(),
    }
    expect(translateTurnSource(event)).toEqual([
      {
        kind: 'fcall-start',
        functionId: 'shell::fs::write',
        input: { path: '/tmp/x' },
        pendingApproval: true,
        functionCallId: 'fc-1',
        sessionId: 'sess-a',
      },
    ])
  })

  it('defaults input to {} when the record omits arguments_excerpt', () => {
    const event: TurnSourceEvent = {
      kind: 'approval-created',
      record: pendingRecord({ arguments_excerpt: undefined }),
    }
    expect(translateTurnSource(event)).toEqual([
      {
        kind: 'fcall-start',
        functionId: 'shell::fs::write',
        input: {},
        pendingApproval: true,
        functionCallId: 'fc-1',
        sessionId: 'sess-a',
      },
    ])
  })

  it('maps a filesystem_access record to fcall-start carrying filesystemAccess', () => {
    const event: TurnSourceEvent = {
      kind: 'approval-created',
      record: pendingRecord({
        kind: 'filesystem_access',
        function_id: 'shell::fs::read',
        access_request: {
          requested_root: '/abs/existing/dir',
          attempted_path: '/abs/existing/dir/secret.txt',
          error_code: 'S215',
        },
      }),
    }
    expect(translateTurnSource(event)).toEqual([
      {
        kind: 'fcall-start',
        functionId: 'shell::fs::read',
        input: { path: '/tmp/x' },
        pendingApproval: true,
        functionCallId: 'fc-1',
        sessionId: 'sess-a',
        filesystemAccess: {
          requestedRoot: '/abs/existing/dir',
          attemptedPath: '/abs/existing/dir/secret.txt',
          errorCode: 'S215',
        },
      },
    ])
  })

  it('omits filesystemAccess for a plain function-call record (kind absent)', () => {
    const event: TurnSourceEvent = {
      kind: 'approval-created',
      record: pendingRecord(),
    }
    const [translated] = translateTurnSource(event)
    expect(translated).not.toHaveProperty('filesystemAccess')
  })

  it('omits filesystemAccess when kind is filesystem_access but access_request.requested_root is missing', () => {
    const event: TurnSourceEvent = {
      kind: 'approval-created',
      record: pendingRecord({ kind: 'filesystem_access' }),
    }
    const [translated] = translateTurnSource(event)
    expect(translated).not.toHaveProperty('filesystemAccess')
  })

  it('maps approval-resolved to fcall-approval-cleared (result arrives via the transcript)', () => {
    const resolved: PendingResolvedEvent = {
      function_call_id: 'fc-1',
      function_id: 'shell::fs::write',
      session_id: 'sess-a',
      turn_id: 't-1',
      outcome: 'allow',
      resolved_at: 5,
    }
    const event: TurnSourceEvent = {
      kind: 'approval-resolved',
      event: resolved,
    }
    expect(translateTurnSource(event)).toEqual([
      { kind: 'fcall-approval-cleared', functionCallId: 'fc-1', running: true },
    ])
  })

  it('does not flip a denied call to running on approval-resolved', () => {
    const resolved: PendingResolvedEvent = {
      function_call_id: 'fc-1',
      function_id: 'shell::fs::write',
      session_id: 'sess-a',
      turn_id: 't-1',
      outcome: 'deny',
      resolved_at: 5,
    }
    const event: TurnSourceEvent = {
      kind: 'approval-resolved',
      event: resolved,
    }
    expect(translateTurnSource(event)).toEqual([
      {
        kind: 'fcall-approval-cleared',
        functionCallId: 'fc-1',
        running: false,
      },
    ])
  })
})

describe('translateTurnSource — turn-completed', () => {
  function completed(over: Partial<TurnCompletedEvent>): TurnSourceEvent {
    return {
      kind: 'turn-completed',
      event: {
        session_id: 'sess-a',
        turn_id: 't-1',
        status: 'completed',
        timestamp: 0,
        ...over,
      },
    }
  }

  it('maps a clean completion to assistant-end only', () => {
    expect(translateTurnSource(completed({ status: 'completed' }))).toEqual([
      { kind: 'assistant-end' },
    ])
  })

  it('emits an aborted stop-reason before assistant-end on cancellation', () => {
    expect(
      translateTurnSource(
        completed({ status: 'cancelled', reason: 'stopped' }),
      ),
    ).toEqual([
      { kind: 'stop-reason', reason: 'aborted', message: 'stopped' },
      { kind: 'assistant-end' },
    ])
  })

  it('surfaces the failure message as an error stop-reason', () => {
    expect(
      translateTurnSource(
        completed({ status: 'failed', result_error: 'provider exploded' }),
      ),
    ).toEqual([
      { kind: 'stop-reason', reason: 'error', message: 'provider exploded' },
      { kind: 'assistant-end' },
    ])
  })

  it('omits the message when a failed turn carries no reason', () => {
    expect(translateTurnSource(completed({ status: 'failed' }))).toEqual([
      { kind: 'stop-reason', reason: 'error', message: undefined },
      { kind: 'assistant-end' },
    ])
  })

  it('flags only turn-completed as terminal', () => {
    expect(isTerminalSource(completed({ status: 'completed' }))).toBe(true)
    expect(
      isTerminalSource({ kind: 'approval-created', record: pendingRecord() }),
    ).toBe(false)
  })
})
