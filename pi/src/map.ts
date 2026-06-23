/**
 * Map Pi AgentSession events to the AgentEvent wire subset. One Pi "turn"
 * (pi::run call) becomes:
 *
 *   message_end (assistant)  -> message_complete
 *   tool_execution_start     -> function_execution_start
 *   tool_execution_end       -> function_execution_end
 *   turn_end                 -> turn_end
 *   agent_end                -> agent_end
 *
 * Pi surfaces tool calls and results as dedicated tool_execution_* events with
 * structured fields, so the assistant message only contributes text/thinking.
 */

import type {
  AgentMessage,
  AssistantMessage,
  ContentBlock,
  FunctionResultMessage,
  Usage,
} from './types.js';

type PiContentBlock = {
  type?: string;
  text?: string;
  thinking?: string;
  reasoning?: string;
  content?: unknown;
};

type PiMessage = { role?: string; content?: unknown };

export type ToolCallIndex = Map<string, { function_id: string; started_at: number }>;

export function toolFunctionId(name: string): string {
  return name.startsWith('mcp__') ? name.replace(/^mcp__/, '').replace(/__/g, '::') : `pi::${name}`;
}

/** Extract the text/thinking blocks from a Pi assistant message. */
export function mapMessageContent(message: unknown): ContentBlock[] {
  const content = (message as PiMessage)?.content;
  if (typeof content === 'string') return content ? [{ type: 'text', text: content }] : [];
  if (!Array.isArray(content)) return [];
  const out: ContentBlock[] = [];
  for (const raw of content as PiContentBlock[]) {
    if (typeof raw === 'string') {
      if (raw) out.push({ type: 'text', text: raw });
      continue;
    }
    if (raw?.type === 'text' && raw.text) out.push({ type: 'text', text: raw.text });
    else if (raw?.type === 'thinking' || raw?.type === 'reasoning') {
      const text = raw.thinking ?? raw.reasoning ?? '';
      if (text) out.push({ type: 'thinking', text });
    }
  }
  return out;
}

/** Normalize a Pi tool result (string | block array | object) to wire content. */
export function mapToolResultContent(raw: unknown): ContentBlock[] {
  if (typeof raw === 'string') return [{ type: 'text', text: raw }];
  if (Array.isArray(raw)) {
    return raw.map((b) => {
      if (typeof b === 'object' && b !== null) {
        const blk = b as PiContentBlock;
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
    provider: 'pi',
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

/** Map Pi SessionStats tokens (input/output/cacheRead/cacheWrite) to wire Usage. */
export function mapUsage(raw: unknown): Usage | null {
  if (typeof raw !== 'object' || raw === null) return null;
  const u = raw as Record<string, number | undefined>;
  return {
    input_tokens: u.input ?? u.input_tokens ?? 0,
    output_tokens: u.output ?? u.output_tokens ?? 0,
    cache_read_tokens: u.cacheRead ?? u.cache_read_tokens ?? 0,
    cache_write_tokens: u.cacheWrite ?? u.cache_write_tokens ?? 0,
  };
}

export function lastAssistant(messages: AgentMessage[]): AgentMessage {
  if (messages.length === 0) return makeAssistantMessage([], '', null);
  for (let i = messages.length - 1; i >= 0; i--) {
    if (messages[i].role === 'assistant') return messages[i];
  }
  return messages[messages.length - 1];
}
