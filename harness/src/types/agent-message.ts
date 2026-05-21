/**
 * Agent transcript message types. Mirrors
 * `harness/crates/harness-types/src/agent_message.rs`.
 */

import type { ContentBlock } from './content.js';
import type { ErrorKind, StopReason, Usage } from './stream-event.js';

export type UserMessage = {
  role: 'user';
  content: ContentBlock[];
  timestamp: number;
};

export type AssistantMessage = {
  role: 'assistant';
  content: ContentBlock[];
  stop_reason: StopReason;
  error_message?: string | null;
  error_kind?: ErrorKind | null;
  usage?: Usage | null;
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

export type CustomMessage = {
  role: 'custom';
  custom_type: string;
  content: ContentBlock[];
  display?: string;
  details?: unknown;
  timestamp: number;
};

export type AgentMessage = UserMessage | AssistantMessage | FunctionResultMessage | CustomMessage;

export function emptyAssistant(provider = '', model = ''): AssistantMessage {
  return {
    role: 'assistant',
    content: [],
    stop_reason: 'end',
    error_message: null,
    error_kind: null,
    usage: null,
    model,
    provider,
    timestamp: Date.now(),
  };
}

export type AgentSessionState = {
  session_id: string;
  model: string;
  thinking_level?: string;
  abort_signal?: boolean;
};

export type AgentContext = {
  system_prompt?: string;
  messages: AgentMessage[];
  functions: import('./function.js').AgentFunction[];
};
