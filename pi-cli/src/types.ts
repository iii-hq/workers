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

/**
 * What the pi extension posts. pi has extensions where Claude Code has shell
 * hooks, so the names are pi's own event names and the payload is flat.
 */
export type PiEvent = {
  event?: string;
  session_id?: string;
  cwd?: string;
  prompt?: string;
  tool?: string;
  call_id?: string;
  args?: Record<string, unknown>;
  result?: unknown;
  is_error?: boolean;
};
