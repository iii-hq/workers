/**
 * Map Claude Agent SDK messages to the AgentEvent wire subset. One Claude
 * Code "turn" (claude::run call) becomes:
 *
 *   assistant msg            -> message_complete (+ function_execution_start per tool_use)
 *   user msg w/ tool_result  -> function_execution_end
 *   result msg               -> turn_end + agent_end
 */

import type {
  AgentMessage,
  AssistantMessage,
  ContentBlock,
  FunctionResultMessage,
  Usage,
} from './types.js';

type SdkContentBlock = {
  type: string;
  text?: string;
  thinking?: string;
  id?: string;
  name?: string;
  input?: unknown;
  tool_use_id?: string;
  content?: unknown;
  is_error?: boolean;
};

export type ToolCallIndex = Map<string, { function_id: string; started_at: number }>;

export function toolFunctionId(name: string): string {
  return name.startsWith('mcp__')
    ? name.replace(/^mcp__/, '').replace(/__/g, '::')
    : `claude-code::${name}`;
}

export function mapAssistantContent(
  blocks: SdkContentBlock[],
  calls: ToolCallIndex,
): ContentBlock[] {
  const out: ContentBlock[] = [];
  for (const b of blocks) {
    if (b.type === 'text' && b.text) out.push({ type: 'text', text: b.text });
    else if (b.type === 'thinking' && b.thinking) out.push({ type: 'thinking', text: b.thinking });
    else if (b.type === 'tool_use' && b.id && b.name) {
      const function_id = toolFunctionId(b.name);
      calls.set(b.id, { function_id, started_at: Date.now() });
      out.push({ type: 'function_call', id: b.id, function_id, arguments: b.input ?? {} });
    }
  }
  return out;
}

export function mapToolResultContent(raw: unknown): ContentBlock[] {
  if (typeof raw === 'string') return [{ type: 'text', text: raw }];
  if (Array.isArray(raw)) {
    return raw.map((b) => {
      if (typeof b === 'object' && b !== null) {
        const blk = b as SdkContentBlock;
        return {
          type: 'text' as const,
          text: blk.type === 'text' ? (blk.text ?? '') : JSON.stringify(blk),
        };
      }
      return { type: 'text' as const, text: JSON.stringify(b ?? null) };
    });
  }
  return [{ type: 'text', text: JSON.stringify(raw ?? null) }];
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
    provider: 'claude-code',
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
    cache_read_tokens: u.cache_read_input_tokens ?? 0,
    cache_write_tokens: u.cache_creation_input_tokens ?? 0,
  };
}

export function lastAssistant(messages: AgentMessage[]): AgentMessage {
  if (messages.length === 0) return makeAssistantMessage([], '', null);
  for (let i = messages.length - 1; i >= 0; i--) {
    if (messages[i].role === 'assistant') return messages[i];
  }
  return messages[messages.length - 1];
}
