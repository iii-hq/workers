/**
 * Wire types for the AgentEvent subset this worker emits onto the
 * `agent::events` stream. Mirrors `claude-code/src/types.ts` and
 * `harness/src/types/*`, so a terminal turn renders in the console like any
 * other agent worker's turn.
 */

export type TextContent = { type: 'text'; text: string };
export type FunctionCallContent = {
  type: 'function_call';
  id: string;
  function_id: string;
  arguments: unknown;
};
export type ContentBlock = TextContent | FunctionCallContent;

export type UserMessage = { role: 'user'; content: ContentBlock[]; timestamp: number };

export type AssistantMessage = {
  role: 'assistant';
  content: ContentBlock[];
  stop_reason: string;
  error_message?: string | null;
  usage?: null;
  model: string;
  provider: string;
  timestamp: number;
};

export type FunctionResultMessage = {
  role: 'function_result';
  function_call_id: string;
  function_id: string;
  content: ContentBlock[];
  details: unknown;
  is_error: boolean;
  timestamp: number;
};

export type AgentMessage = UserMessage | AssistantMessage | FunctionResultMessage;

export type AgentEvent =
  | { type: 'agent_end'; messages: AgentMessage[] }
  | { type: 'turn_end'; message: AgentMessage; function_results: FunctionResultMessage[] }
  | { type: 'message_complete'; message: AgentMessage; body_streamed?: boolean }
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
      result: { content: ContentBlock[]; details: unknown };
      is_error: boolean;
      duration_ms: number;
    };

/** The Claude Code hook payload, as the workspace hooks post it. */
export type HookEvent = {
  hook_event_name?: string;
  session_id?: string;
  cwd?: string;
  prompt?: string;
  tool_name?: string;
  tool_use_id?: string;
  tool_input?: Record<string, unknown>;
  tool_response?: unknown;
};
