import type { AssistantMessage, FunctionResultMessage } from '../types/agent-message.js';
import type { FunctionCall } from '../types/function.js';

export type TurnState =
  | 'provisioning'
  | 'awaiting_assistant'
  | 'assistant_streaming'
  | 'assistant_finished'
  | 'function_prepare'
  | 'function_execute'
  | 'function_awaiting_approval'
  | 'function_finalize'
  | 'steering_check'
  | 'tearing_down'
  | 'stopped';

export type AwaitingApprovalEntry = {
  function_call_id: string;
  function_id: string;
  args: unknown;
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
  return rec.state === 'stopped';
}

export const messagesKey = (sid: string) => `session/${sid}/messages`;
export const turnStateKey = (sid: string) => `session/${sid}/turn_state`;
export const runRequestKey = (sid: string) => `session/${sid}/run_request`;
export const cwdKey = (sid: string) => `session/${sid}/cwd`;
export const cwdIndexKey = (hash: string) => `harness/cwd/${hash}/last_session_id`;
export const sandboxIdKey = (sid: string) => `session/${sid}/sandbox_id`;
export const functionSchemasKey = (sid: string) => `session/${sid}/function_schemas`;
export const toolSchemasKey = (sid: string) => `session/${sid}/tool_schemas`;
export const lastSessionTreeLenKey = (sid: string) => `session/${sid}/session_tree_mirror_len`;
export const lastCompactionAtKey = (sid: string) => `session/${sid}/last_compaction_at`;
export const lastCompactionConsumedAtKey = (sid: string) =>
  `session/${sid}/last_compaction_consumed_at`;
export const eventCounterKey = (sid: string) => `session/${sid}/event_counter`;
export const abortSignalKey = (sid: string) => `session/${sid}/abort_signal`;
