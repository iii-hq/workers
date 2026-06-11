/**
 * Map Codex SDK thread items to the AgentEvent wire subset. One Codex turn
 * (codex::run call) becomes:
 *
 *   agent_message / reasoning item -> message_complete
 *   command_execution / file_change / mcp_tool_call / web_search item
 *     -> function_execution_start (item.started) + function_execution_end
 *   turn.completed -> turn_end + agent_end
 */

import type {
  AgentMessage,
  AssistantMessage,
  ContentBlock,
  FunctionResultMessage,
  Usage,
} from './types.js';

/** Loose view of a Codex SDK ThreadItem — the SDK union, untyped at the seam. */
export type CodexItem = {
  id: string;
  type: string;
  text?: string;
  command?: string;
  aggregated_output?: string;
  exit_code?: number;
  status?: string;
  changes?: unknown;
  server?: string;
  tool?: string;
  arguments?: unknown;
  result?: unknown;
  error?: { message?: string };
  query?: string;
};

const EXEC_ITEM_TYPES = new Set([
  'command_execution',
  'file_change',
  'mcp_tool_call',
  'web_search',
]);

export function isExecItem(item: CodexItem): boolean {
  return EXEC_ITEM_TYPES.has(item.type);
}

export function functionIdForItem(item: CodexItem): string {
  switch (item.type) {
    case 'command_execution':
      return 'codex::shell';
    case 'file_change':
      return 'codex::apply_patch';
    case 'web_search':
      return 'codex::web_search';
    case 'mcp_tool_call':
      return `${item.server ?? 'mcp'}::${item.tool ?? 'tool'}`;
    default:
      return `codex::${item.type}`;
  }
}

export function argsForItem(item: CodexItem): unknown {
  switch (item.type) {
    case 'command_execution':
      return { command: item.command ?? '' };
    case 'file_change':
      return { changes: item.changes ?? [] };
    case 'web_search':
      return { query: item.query ?? '' };
    case 'mcp_tool_call':
      return item.arguments ?? {};
    default:
      return {};
  }
}

export function resultContentForItem(item: CodexItem): ContentBlock[] {
  switch (item.type) {
    case 'command_execution':
      return [{ type: 'text', text: item.aggregated_output ?? '' }];
    case 'file_change':
      return [{ type: 'text', text: JSON.stringify(item.changes ?? []) }];
    case 'web_search':
      return [{ type: 'text', text: item.query ?? '' }];
    case 'mcp_tool_call':
      return [
        {
          type: 'text',
          text: item.error?.message ?? JSON.stringify(item.result ?? null),
        },
      ];
    default:
      return [{ type: 'text', text: JSON.stringify(item) }];
  }
}

export function isErrorItem(item: CodexItem): boolean {
  if (item.status === 'failed') return true;
  return (
    item.type === 'command_execution' && typeof item.exit_code === 'number' && item.exit_code !== 0
  );
}

export function makeAssistantMessage(
  content: ContentBlock[],
  model: string,
  usage: Usage | null,
  stop_reason = 'end',
): AssistantMessage {
  return {
    role: 'assistant',
    content,
    stop_reason,
    error_message: null,
    usage,
    model,
    provider: 'codex',
    timestamp: Date.now(),
  };
}

export function makeFunctionResult(
  function_call_id: string,
  function_id: string,
  content: ContentBlock[],
  is_error: boolean,
): FunctionResultMessage {
  return {
    role: 'function_result',
    function_call_id,
    function_id,
    content,
    details: null,
    is_error,
    timestamp: Date.now(),
  };
}

export function mapUsage(raw: unknown): Usage | null {
  if (typeof raw !== 'object' || raw === null) return null;
  const u = raw as Record<string, number | undefined>;
  return {
    input_tokens: u.input_tokens ?? 0,
    output_tokens: u.output_tokens ?? 0,
    cache_read_tokens: u.cached_input_tokens,
    reasoning_tokens: u.reasoning_output_tokens,
  };
}

export function lastAssistant(messages: AgentMessage[]): AgentMessage {
  if (messages.length === 0) return makeAssistantMessage([], '', null);
  for (let i = messages.length - 1; i >= 0; i--) {
    if (messages[i].role === 'assistant') return messages[i];
  }
  return messages[messages.length - 1];
}
