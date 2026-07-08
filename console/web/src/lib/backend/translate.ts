/**
 * Pure translator from the harness/approval trigger payloads the console binds
 * around a turn to console/web's `StreamEvent` contract.
 *
 * Transcript CONTENT (assistant text, thinking, function-call blocks, function
 * results) renders from `session::message-added` / `session::message-updated`
 * reconciled by the conversations layer — NOT from this translator. These
 * trigger events carry only the ephemeral turn surface:
 *
 *   - `approval::pending-created`  → `fcall-start { pendingApproval: true }`
 *   - `approval::pending-resolved` → `fcall-approval-cleared`
 *   - `harness::turn-completed`    → `assistant-end` (+ `stop-reason` when the
 *                                    turn failed / was cancelled)
 *
 * The translator is stateless: the new gate triggers are already discrete
 * create/resolve events, so no list-diffing (the old `turn_state_changed`
 * mirror) is needed.
 */

import type {
  PendingApprovalRecord,
  PendingResolvedEvent,
  TurnCompletedEvent,
} from '@/types/iii-agent-event'
import type { StreamEvent } from './types'

/** The discriminated union of trigger events `realStream` feeds the translator. */
export type TurnSourceEvent =
  | { kind: 'approval-created'; record: PendingApprovalRecord }
  | { kind: 'approval-resolved'; event: PendingResolvedEvent }
  | { kind: 'turn-completed'; event: TurnCompletedEvent }

/** True once this event ends the turn (the generator should stop after it). */
export function isTerminalSource(event: TurnSourceEvent): boolean {
  return event.kind === 'turn-completed'
}

export function translateTurnSource(event: TurnSourceEvent): StreamEvent[] {
  switch (event.kind) {
    case 'approval-created': {
      const { record } = event
      const accessRequest =
        record.kind === 'filesystem_access' &&
        record.access_request?.requested_root
          ? record.access_request
          : undefined
      return [
        {
          kind: 'fcall-start',
          functionId: record.function_id,
          input: record.arguments_excerpt ?? {},
          pendingApproval: true,
          functionCallId: record.function_call_id,
          sessionId: record.session_id,
          ...(accessRequest
            ? {
                filesystemAccess: {
                  requestedRoot: accessRequest.requested_root,
                  attemptedPath: accessRequest.attempted_path,
                  errorCode: accessRequest.error_code,
                },
              }
            : {}),
        },
      ]
    }

    case 'approval-resolved':
      // The released call's result (or deny error) arrives via the session
      // transcript; here we clear the pending prompt on its card and, when
      // allowed, flip it to running while the released call executes.
      return [
        {
          kind: 'fcall-approval-cleared',
          functionCallId: event.event.function_call_id,
          running: event.event.outcome === 'allow',
        },
      ]

    case 'turn-completed':
      return translateTurnCompleted(event.event)
  }
}

function translateTurnCompleted(event: TurnCompletedEvent): StreamEvent[] {
  const out: StreamEvent[] = []
  if (event.status === 'cancelled') {
    out.push({ kind: 'stop-reason', reason: 'aborted', message: event.reason })
  } else if (event.status === 'failed') {
    const message = event.result_error ?? event.reason
    out.push({
      kind: 'stop-reason',
      reason: 'error',
      message: message && message.length > 0 ? message : undefined,
    })
  }
  out.push({ kind: 'assistant-end' })
  return out
}
