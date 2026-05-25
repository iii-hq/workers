/**
 * Domain types for the function_execute pipeline.
 */

import type { FunctionCall, FunctionResult } from '../../types/function.js';

/** Exactly one execution route per prepared call: dispatch, pre_approved, or synthetic. */
export type PreparedCall =
  | { route: 'dispatch'; call: FunctionCall }
  | { route: 'pre_approved'; call: FunctionCall }
  | { route: 'synthetic'; call: FunctionCall; result: FunctionResult };

export type ExecutedCall = {
  call: FunctionCall;
  result: FunctionResult;
  is_error: boolean;
  duration_ms: number;
};

/** Durable mid-batch state persisted on TurnStateRecord.work. */
export type FunctionBatchWork = {
  prepared: readonly PreparedCall[];
  executed: Record<string, ExecutedCall>;
};

export type PendingApproval = {
  function_call_id: string;
  function_id: string;
  args: FunctionCall['arguments'];
};

/** Batch loop outcome — explicit control flow instead of early return + void. */
export type BatchOutcome =
  | { kind: 'completed'; work: FunctionBatchWork }
  | { kind: 'parked'; work: FunctionBatchWork; pending: PendingApproval };

export type RunOneCallResult =
  | { kind: 'skipped' }
  | { kind: 'executed'; entry: ExecutedCall }
  | { kind: 'pending'; call: FunctionCall };

export type ResolveCallResult =
  | { kind: 'pending' }
  | { kind: 'resolved'; result: FunctionResult; is_error: boolean };

/** Extract the FunctionCall from any PreparedCall variant. */
export function preparedCallId(prepared: PreparedCall): string {
  return prepared.call.id;
}

/** Empty durable work for a fresh batch. */
export function emptyBatchWork(prepared: readonly PreparedCall[]): FunctionBatchWork {
  return { prepared, executed: {} };
}
