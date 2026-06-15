/**
 * TurnState + TurnStateRecord types and parsers.
 *
 * Persistence uses semantic iii scopes (`turn_state`, `run_request`, …). Conversation
 * history lives in the session-manager worker (`session::*`) and is reconstructed at
 * read time, keyed by `session_id`. Recovery lists scope `turn_state` via {@link parseTurnStateRecord}.
 */

import { z } from 'zod';
import { uuidLike } from '../runtime/ids.js';
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
  /**
   * Unique id for this run, generated at `newRecord`. Threaded through the
   * pre_dispatch hook contract, `harness::function::resolve`, and
   * `harness::turn-completed` so sibling workers (the approval-gate) can key
   * their records to one turn. Never contains `/` (reserved state-key
   * separator on the sibling side).
   */
  turn_id: string;
  turn_count: number;
  max_turns?: number;
  /**
   * Caller's per-send id (console `msg-<uuid>`). Seeds the deterministic
   * session-manager entry ids for this run (see runtime/session.ts runKey).
   */
  message_id?: string;
  function_results: FunctionResultMessage[];
  turn_end_emitted: boolean;
  started_at_ms: number;
  updated_at_ms: number;
  /** Set during assistant_streaming when message_update deltas were emitted. */
  assistant_body_streamed?: boolean;
  /**
   * Full catalog entry for this turn's model, resolved once at `provisioning`
   * and read by the assemble pre-flight (inline limits for `context::assemble`)
   * and provider streaming instead of re-fetching `models::get`. Optional:
   * absent on records persisted before this field existed (and on a cold
   * catalog) — readers fall back accordingly. The model is fixed for a turn, so
   * no invalidation is needed. Excluded from {@link toView}, so it never bloats
   * turn_state events.
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

const TURN_STATES = [
  'provisioning',
  'assistant_streaming',
  'function_execute',
  'function_awaiting_approval',
  'finishing',
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
  if (!result.success) return null;
  const rec = result.data as TurnStateRecord;
  // Records persisted before turn_id existed: backfill deterministically
  // (stable across re-parses) so in-flight turns survive the deploy.
  if (typeof rec.turn_id !== 'string' || rec.turn_id.length === 0) {
    rec.turn_id = `legacy-${rec.session_id}`;
  }
  return rec;
}

export function newRecord(
  session_id: string,
  max_turns?: number,
  message_id?: string,
): TurnStateRecord {
  const now = Date.now();
  return {
    session_id,
    turn_id: `t_${uuidLike()}`,
    state: 'provisioning',
    turn_count: 0,
    max_turns,
    ...(message_id ? { message_id } : {}),
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
