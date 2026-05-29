/**
 * Typed dependency ports and domain types for function_awaiting_approval.
 */

import { ApprovalDecisionSchema, STATE_SCOPE } from '../../approval-gate/schemas.js';
import type { ISdk } from '../../runtime/iii.js';
import type { z } from 'zod';
export type ApprovalDecision = z.infer<typeof ApprovalDecisionSchema>;

/** Decode stored approval decision from `state::get` (scope `approvals`). */
export function parseApprovalDecision(value: unknown): ApprovalDecision | null {
  const parsed = ApprovalDecisionSchema.safeParse(value);
  return parsed.success ? parsed.data : null;
}

export type AwaitingApprovalPorts = {
  readDecision(session_id: string, function_call_id: string): Promise<ApprovalDecision | null>;
};

export function createAwaitingApprovalPorts(iii: ISdk): AwaitingApprovalPorts {
  return {
    async readDecision(session_id, function_call_id) {
      const key = `${session_id}/${function_call_id}`;
      const raw = await iii.trigger<unknown, unknown>({
        function_id: 'state::get',
        payload: { scope: STATE_SCOPE, key },
      });
      return parseApprovalDecision(raw);
    },
  };
}
