/**
 * TurnState + TurnStateRecord types and parsers.
 *
 * Persistence uses semantic iii scopes (`turn_state`, `messages`, `run_request`, …)
 * keyed by `session_id`. Recovery lists scope `turn_state` via {@link parseTurnStateRecord}.
 */

import { z } from 'zod';
import type { AssistantMessage, FunctionResultMessage } from '../types/agent-message.js';
import type {
  ExecutedCall,
  FunctionBatchWork,
  PreparedCall,
} from './function-execute/types.js';

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

export type AwaitingApprovalEntry = {
  function_call_id: string;
  function_id: string;
  args: unknown;
};

/** Durable mid-batch work for function_execute. */
export type TurnWork = FunctionBatchWork;

export type { ExecutedCall, FunctionBatchWork, PreparedCall };

export type TurnStateRecord = {
  session_id: string;
  state: TurnState;
  turn_count: number;
  max_turns?: number;
  last_assistant?: AssistantMessage | null;
  function_results: FunctionResultMessage[];
  turn_end_emitted: boolean;
  started_at_ms: number;
  updated_at_ms: number;
  awaiting_approval?: AwaitingApprovalEntry[];
  /** Set during assistant_streaming when message_update deltas were emitted. */
  assistant_body_streamed?: boolean;
  work?: TurnWork;
  error?: { kind: string; message: string };
};

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
