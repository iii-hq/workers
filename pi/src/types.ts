/**
 * Wire types for the AgentEvent subset this worker emits onto the
 * `agent::events` stream. Mirrors harness/src/types/* in iii-hq/workers so
 * the console and acp worker render Pi turns like any other agent worker.
 */

export type TextContent = { type: 'text'; text: string };
export type ThinkingContent = { type: 'thinking'; text: string; signature?: string };
export type FunctionCallContent = {
  type: 'function_call';
  id: string;
  function_id: string;
  arguments: unknown;
};
export type FunctionResultContent = {
  type: 'function_result';
  function_call_id: string;
  content: ContentBlock[];
  is_error?: boolean;
};
export type ContentBlock =
  | TextContent
  | ThinkingContent
  | FunctionCallContent
  | FunctionResultContent;

export type Usage = {
  input_tokens: number;
  output_tokens: number;
  cache_read_tokens?: number;
  cache_write_tokens?: number;
};

export type AssistantMessage = {
  role: 'assistant';
  content: ContentBlock[];
  stop_reason: string;
  error_message?: string | null;
  usage?: Usage | null;
  model: string;
  provider: string;
  timestamp: number;
};

export type UserMessage = { role: 'user'; content: ContentBlock[]; timestamp: number };

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

export type SessionRecord = {
  session_id: string;
  pi_session_id: string | null;
  session_file: string | null;
  cwd: string;
  model: string;
  status: 'working' | 'done' | 'error';
  turns: number;
  total_cost_usd: number;
  usage: Usage | null;
  updated_at_ms: number;
};
