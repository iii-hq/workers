/**
 * Approval-gate lifecycle events. Mirrors the `approval_requested`,
 * and `approval_resolved` payloads written via `write_event`.
 */

import { uuidLike } from '../runtime/ids.js';
import type { ISdk } from '../runtime/iii.js';
import { streamSet } from '../runtime/stream.js';

const STREAM_NAME = 'agent::events';

async function writeEvent(iii: ISdk, session_id: string, data: unknown): Promise<void> {
  await streamSet(iii, {
    stream_name: STREAM_NAME,
    group_id: session_id,
    item_id: `approval-${uuidLike()}`,
    data,
  });
}

export type ApprovalRequestedPayload = {
  function_call_id: string;
  function_id: string;
  args: unknown;
};

export async function emitApprovalRequested(
  iii: ISdk,
  session_id: string,
  payload: ApprovalRequestedPayload,
): Promise<void> {
  await writeEvent(iii, session_id, {
    type: 'approval_requested',
    function_call_id: payload.function_call_id,
    tool_call_id: payload.function_call_id,
    function_id: payload.function_id,
    tool_name: payload.function_id,
    args: payload.args,
  });
}

export type ApprovalResolvedPayload = {
  function_call_id: string;
  decision: 'allow' | 'deny';
  reason: string | null;
};

export async function emitApprovalResolved(
  iii: ISdk,
  session_id: string,
  payload: ApprovalResolvedPayload,
): Promise<void> {
  await writeEvent(iii, session_id, {
    type: 'approval_resolved',
    function_call_id: payload.function_call_id,
    tool_call_id: payload.function_call_id,
    decision: payload.decision,
    reason: payload.reason,
  });
}
