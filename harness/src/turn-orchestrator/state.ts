/**
 * TurnState + TurnStateRecord types and constructors. Persisted under iii scope
 * `turn_state` keyed by session_id; conversation history lives in `session-tree::*`.
 * State is cleared on deploy, so records are read back typed (no re-parsing).
 */

import type { Model } from '../types/model.js';
import type { AssistantMessage, FunctionResultMessage } from '../types/agent-message.js';
import type { ExecutedCall, FunctionBatchWork, PreparedCall } from './function-execute/types.js';

/** Shared iii scope names for turn-orchestrator persistence (key = session_id). */
export const TURN_STATE_SCOPE = 'turn_state';
export const RUN_REQUEST_SCOPE = 'run_request';

export type TurnState =
  | 'provisioning'
  | 'assistant_streaming'
  | 'function_execute'
  | 'function_awaiting_approval'
  | 'finishing'
  | 'stopped'
  | 'failed';

export const FUNCTION_BATCH_STATES = ['function_execute', 'function_awaiting_approval'] as const;
export type FunctionBatchState = (typeof FUNCTION_BATCH_STATES)[number];

export type AwaitingApprovalEntry = {
  function_call_id: string;
  function_id: string;
  args: unknown;
};

/** Durable mid-batch work for function_execute. */
export type TurnWork = FunctionBatchWork;

export type { ExecutedCall, FunctionBatchWork, PreparedCall };

type TurnStateRecordCore = {
  session_id: string;
  turn_count: number;
  max_turns?: number;
  function_results: FunctionResultMessage[];
  turn_end_emitted: boolean;
  started_at_ms: number;
  updated_at_ms: number;
  /** Set during assistant_streaming when message_update deltas were emitted. */
  assistant_body_streamed?: boolean;
  /**
   * Full catalog entry for this turn's model, resolved once at `provisioning`
   * and read by preflight, provider streaming, and turn-end compaction instead
   * of each re-fetching `models::get`. Optional: absent on records persisted
   * before this field existed (and on a cold catalog) — every reader falls back
   * to a live fetch. The model is fixed for a turn, so no invalidation is
   * needed. Excluded from {@link toView}, so it never bloats turn_state events.
   */
  model_meta?: Model;
  error?: { kind: string; message: string };
};

/** Required fields while in function_execute or function_awaiting_approval. */
export type FunctionBatchTurnRecord = TurnStateRecordCore & {
  state: FunctionBatchState;
  last_assistant: AssistantMessage;
  work: TurnWork;
  awaiting_approval: AwaitingApprovalEntry[];
};

/** Persisted shape while in assistant_streaming (last_assistant set mid-handler). */
export type AssistantStreamingTurnRecord = TurnStateRecordCore & {
  state: 'assistant_streaming';
  last_assistant?: AssistantMessage | null;
  work?: TurnWork;
  awaiting_approval?: AwaitingApprovalEntry[];
};

type OtherTurnState = Exclude<TurnState, FunctionBatchState | 'assistant_streaming'>;

export type TurnStateRecord =
  | FunctionBatchTurnRecord
  | AssistantStreamingTurnRecord
  | (TurnStateRecordCore & {
      state: OtherTurnState;
      last_assistant?: AssistantMessage | null;
      work?: TurnWork;
      awaiting_approval?: AwaitingApprovalEntry[];
    });

export function newRecord(session_id: string, max_turns?: number): TurnStateRecord {
  const now = Date.now();
  return {
    session_id,
    state: 'provisioning',
    turn_count: 0,
    max_turns,
    last_assistant: null,
    function_results: [],
    turn_end_emitted: false,
    started_at_ms: now,
    updated_at_ms: now,
  };
}

export function transitionTo(rec: TurnStateRecord, next: TurnState): void {
  rec.state = next;
  rec.updated_at_ms = Date.now();
}

/** A turn is done once it reaches a terminal state; otherwise it is in flight. */
const TERMINAL_TURN_STATES = new Set<TurnState>(['stopped', 'failed']);

export function isTurnInFlight(rec: TurnStateRecord): boolean {
  return !TERMINAL_TURN_STATES.has(rec.state);
}
