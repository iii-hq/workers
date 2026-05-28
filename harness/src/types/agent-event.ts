/**
 * Stream events emitted on `agent::events`. Mirrors
 * `harness/crates/harness-types/src/agent_event.rs`.
 *
 * The Rust enum is externally tagged (`type: snake_case`) and the wire
 * format is what console/web consumes.
 */

import type { AgentMessage, FunctionResultMessage } from './agent-message.js';
import type { FunctionResult } from './function.js';
import type { AssistantMessageEvent } from './stream-event.js';

export type AgentEvent =
  | { type: 'agent_end'; messages: AgentMessage[] }
  | {
      type: 'turn_end';
      message: AgentMessage;
      function_results: FunctionResultMessage[];
    }
  | {
      type: 'message_update';
      message: AgentMessage;
      llm_event: AssistantMessageEvent;
    }
  | {
      type: 'message_complete';
      message: AgentMessage;
      /** When true, text/thinking were already delivered via message_update. */
      body_streamed?: boolean;
    }
  | {
      type: 'function_execution_start';
      function_call_id: string;
      function_id: string;
      args: unknown;
    }
  | {
      type: 'function_execution_end';
      function_call_id: string;
      function_id: string;
      result: FunctionResult;
      is_error: boolean;
      /** Wall-clock ms between the matching function_execution_start and end.
       *  Reused from persisted ExecutedCall on resumed runs so replayed
       *  calls keep their original timing. */
      duration_ms: number;
    }
  | {
      type: 'turn_state_changed';
      /** Full new turn_state record (the orchestrator's persisted state). */
      new_value: Record<string, unknown>;
      /** Previous record, present on state:updated; absent on state:created. */
      old_value?: Record<string, unknown>;
      event_type: 'state:created' | 'state:updated';
    }
  | {
      /**
       * Emitted by context-compaction after a successful flat-state rewrite.
       * Lets the UI insert a "compaction" marker into the rendered transcript
       * and re-estimate context usage — without this, the UI's CTX bar keeps
       * counting messages the server has already summarised.
       */
      type: 'compaction_done';
      /** 'async' = TurnEnd-driven background; 'sync' = pre-flight in-turn. */
      mode: 'async' | 'sync';
      /** The summary written into the session-tree Compaction entry. */
      summary_text: string;
      /** Estimated tokens of the pre-compaction head (what got summarised). */
      tokens_before: number;
      /** session-tree entry_id of the new Compaction node. */
      compaction_entry_id: string;
      /** First entry_id of the preserved tail; null when nothing was kept. */
      tail_start_id: string | null;
    };
