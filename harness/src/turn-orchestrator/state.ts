/**
 * TurnState + TurnStateRecord + state-key helpers. Mirrors
 * `turn-orchestrator/src/state.rs`.
 */

import type { AssistantMessage, FunctionResultMessage } from '../types/agent-message.js';
import type { FunctionCall, FunctionResult } from '../types/function.js';

export type TurnState =
  | 'provisioning'
  | 'assistant_streaming'
  | 'function_execute'
  | 'function_awaiting_approval'
  | 'steering_check'
  | 'tearing_down'
  | 'stopped'
  | 'failed';

export type AwaitingApprovalEntry = {
  function_call_id: string;
  function_id: string;
  args: unknown;
};

export type PreparedEntry = {
  function_call: FunctionCall;
  blocked: FunctionResult | null;
  pre_approved?: boolean;
};

export type ExecutedEntry = {
  function_call: FunctionCall;
  result: FunctionResult;
  is_error: boolean;
  duration_ms: number;
};

export type TurnWork = {
  batch: PreparedEntry[];
  results: ExecutedEntry[];
};

export type TurnStateRecord = {
  session_id: string;
  state: TurnState;
  turn_count: number;
  max_turns?: number;
  last_assistant?: AssistantMessage | null;
  pending_function_calls: FunctionCall[];
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

export function newRecord(session_id: string, max_turns?: number): TurnStateRecord {
  const now = Date.now();
  return {
    session_id,
    state: 'provisioning',
    turn_count: 0,
    max_turns,
    last_assistant: null,
    pending_function_calls: [],
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

export function isTerminal(rec: TurnStateRecord): boolean {
  return rec.state === 'stopped' || rec.state === 'failed';
}

export const messagesKey = (sid: string) => `session/${sid}/messages`;
export const turnStateKey = (sid: string) => `session/${sid}/turn_state`;
export const runRequestKey = (sid: string) => `session/${sid}/run_request`;
export const lastSessionTreeLenKey = (sid: string) => `session/${sid}/session_tree_mirror_len`;
export const eventCounterKey = (sid: string) => `session/${sid}/event_counter`;
export const abortSignalKey = (sid: string) => `session/${sid}/abort_signal`;
