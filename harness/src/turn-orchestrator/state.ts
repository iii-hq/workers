/**
 * TurnState + TurnStateRecord types and parsers.
 *
 * Persistence uses semantic iii scopes (`turn_state`, `messages`, `run_request`, …)
 * keyed by `session_id`. Recovery lists scope `turn_state` via {@link parseTurnStateRecord}.
 */

import { z } from 'zod';
import type { AssistantMessage, FunctionResultMessage } from '../types/agent-message.js';
import type { ExecutedCall, FunctionBatchWork, PreparedCall } from './function-execute/types.js';

/** Shared iii scope names for turn-orchestrator persistence (key = session_id). */
export const TURN_STATE_SCOPE = 'turn_state';
export const MESSAGES_SCOPE = 'messages';
export const RUN_REQUEST_SCOPE = 'run_request';

export type TurnState =
  | 'provisioning'
  | 'assistant_streaming'
  | 'function_execute'
  | 'function_awaiting_approval'
  | 'steering_check'
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

/** Persisted shape while in steering_check (work cleared on entry from function batch). */
export type SteeringCheckTurnRecord = TurnStateRecordCore & {
  state: 'steering_check';
  last_assistant?: AssistantMessage | null;
  work?: TurnWork;
  awaiting_approval?: AwaitingApprovalEntry[];
};

type OtherTurnState = Exclude<TurnState, FunctionBatchState | 'assistant_streaming' | 'steering_check'>;

export type TurnStateRecord =
  | FunctionBatchTurnRecord
  | AssistantStreamingTurnRecord
  | SteeringCheckTurnRecord
  | (TurnStateRecordCore & {
      state: OtherTurnState;
      last_assistant?: AssistantMessage | null;
      work?: TurnWork;
      awaiting_approval?: AwaitingApprovalEntry[];
    });

const TURN_STATES = [
  'provisioning',
  'assistant_streaming',
  'function_execute',
  'function_awaiting_approval',
  'steering_check',
  'stopped',
  'failed',
] as const satisfies readonly TurnState[];

/** Minimal structural guard for persisted turn_state — nested fields pass through. */
const TurnStateRecordSchema = z
  .object({
    session_id: z.string(),
    state: z.enum(TURN_STATES),
    turn_count: z.number().catch(0),
    function_results: z.array(z.unknown()).catch([]),
    turn_end_emitted: z.boolean().catch(false),
    started_at_ms: z.number().catch(0),
    updated_at_ms: z.number().catch(0),
  })
  .passthrough();

export function parseTurnStateRecord(raw: unknown): TurnStateRecord | null {
  const result = TurnStateRecordSchema.safeParse(raw);
  return result.success ? (result.data as TurnStateRecord) : null;
}

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
