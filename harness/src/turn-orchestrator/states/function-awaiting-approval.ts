/**
 * `turn::function_awaiting_approval`. Read approval decisions and resume execute.
 *
 * **Incoming**: flat `{ session_id }` via FIFO enqueue on `turn-step`.
 * **Outgoing**: `{ ok, from_state, to_state }` on success; stale skip when state drifted.
 */

import { ApprovalResumePayloadSchema, STATE_SCOPE } from '../../approval-gate/schemas.js';
import type { z } from 'zod';
import type { ISdk } from '../../runtime/iii.js';
import type { FunctionResult } from '../../types/function.js';
import { text } from '../../types/content.js';
import * as persistence from '../persistence.js';
import { runTransition } from '../run-transition.js';
import { type TurnStateRecord, transitionTo } from '../state.js';
import { TurnStepPayloadSchema, type TurnStepPayload } from '../schemas.js';

export type ApprovalDecision = z.infer<typeof ApprovalResumePayloadSchema>;

/** Decode stored approval decision from `state::get` (scope `approvals`). */
export function parseApprovalDecision(value: unknown): ApprovalDecision | null {
  const parsed = ApprovalResumePayloadSchema.safeParse(value);
  return parsed.success ? parsed.data : null;
}

async function readDecision(
  iii: ISdk,
  session_id: string,
  function_call_id: string,
): Promise<ApprovalDecision | null> {
  const key = `${session_id}/${function_call_id}`;
  const raw = await iii.trigger<unknown, unknown>({
    function_id: 'state::get',
    payload: { scope: STATE_SCOPE, key },
  });
  return parseApprovalDecision(raw);
}

function denialResultFromDecision(decision: ApprovalDecision): FunctionResult {
  const reason =
    decision.reason ?? (decision.decision === 'aborted' ? 'session_aborted' : 'denied');
  const message =
    decision.decision === 'aborted'
      ? `Function call aborted: ${reason}`
      : `Permission denied by user: ${reason}`;
  return {
    content: [text(message)],
    details: {
      approval_denied: true,
      decision: decision.decision,
      reason,
    },
    terminate: false,
  };
}

export async function handleAwaitingApproval(iii: ISdk, rec: TurnStateRecord): Promise<void> {
  const awaiting = rec.awaiting_approval ?? [];
  if (awaiting.length === 0) {
    transitionTo(rec, 'function_execute');
    return;
  }

  const decisions = await Promise.all(
    awaiting.map((entry) => readDecision(iii, rec.session_id, entry.function_call_id)),
  );

  if (decisions.some((decision) => decision === null)) {
    return;
  }

  const prepared = await persistence.loadPreparedCalls(iii, rec.session_id);
  for (let i = 0; i < awaiting.length; i++) {
    const entry = awaiting[i];
    const decision = decisions[i];
    if (!entry || !decision) continue;
    const idx = prepared.findIndex(
      (preparedEntry) => preparedEntry.function_call.id === entry.function_call_id,
    );
    if (idx < 0) continue;
    const current = prepared[idx];
    if (!current) continue;
    if (decision.decision === 'allow') {
      prepared[idx] = { ...current, pre_approved: true, blocked: null };
    } else {
      prepared[idx] = {
        ...current,
        pre_approved: false,
        blocked: denialResultFromDecision(decision),
      };
    }
  }

  await persistence.savePreparedCalls(iii, rec.session_id, prepared);

  rec.awaiting_approval = [];
  transitionTo(rec, 'function_execute');
}

export function register(iii: ISdk): void {
  iii.registerFunction(
    'turn::function_awaiting_approval',
    async (payload: TurnStepPayload) => {
      const parsed = TurnStepPayloadSchema.parse(payload);
      return runTransition(iii, 'function_awaiting_approval', handleAwaitingApproval, parsed);
    },
    {
      description:
        'Run one durable FSM transition for session in state function_awaiting_approval: read approval decisions and resume.',
    },
  );
}
