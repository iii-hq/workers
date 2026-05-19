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

export type ApprovalDecision = 'allow' | 'deny';

export type AgentEvent =
  | { type: 'agent_start' }
  | { type: 'agent_end'; messages: AgentMessage[] }
  | { type: 'turn_start' }
  | {
      type: 'turn_end';
      message: AgentMessage;
      function_results: FunctionResultMessage[];
    }
  | { type: 'message_start'; message: AgentMessage }
  | {
      type: 'message_update';
      message: AgentMessage;
      llm_event: AssistantMessageEvent;
    }
  | { type: 'message_end'; message: AgentMessage }
  | {
      type: 'function_execution_start';
      function_call_id: string;
      function_id: string;
      args: unknown;
    }
  | {
      type: 'function_execution_update';
      function_call_id: string;
      function_id: string;
      args: unknown;
      partial_result: unknown;
    }
  | {
      type: 'function_execution_end';
      function_call_id: string;
      function_id: string;
      result: FunctionResult;
      is_error: boolean;
    }
  | {
      type: 'approval_requested';
      function_call_id: string;
      function_id: string;
      args: unknown;
    }
  | {
      type: 'approval_resolved';
      function_call_id: string;
      decision: ApprovalDecision;
      reason?: string | null;
    }
  | {
      type: 'turn_state_changed';
      /** Full new turn_state record (the orchestrator's persisted state). */
      new_value: Record<string, unknown>;
      /** Previous record, present on state:updated; absent on state:created. */
      old_value?: Record<string, unknown>;
      event_type: 'state:created' | 'state:updated';
    };
