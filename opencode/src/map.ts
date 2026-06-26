/**
 * Map `opencode run --format json` events to the AgentEvent wire subset.
 *
 * OpenCode emits one JSON object per line. The shapes this worker consumes
 * (captured from a live `opencode run --format json`):
 *
 *   { type: "step_start",  sessionID, part:{ type:"step-start" } }
 *   { type: "text",        sessionID, part:{ type:"text", text } }
 *   { type: "tool_use",    sessionID, part:{ type:"tool", tool, callID,
 *                            state:{ status, input, output, metadata:{exit}, time } } }
 *   { type: "step_finish", sessionID, part:{ type:"step-finish", reason,
 *                            tokens:{ input, output, reasoning, cache:{read,write} }, cost } }
 *
 * One turn becomes:
 *   text          -> message_complete (assistant)
 *   tool_use      -> function_execution_start + function_execution_end
 *   step_finish   -> usage + cost accumulation (surfaced on agent_end + return)
 */

import type { AssistantMessage, ContentBlock, FunctionResultMessage, Usage } from './types.js';

export type OpencodeEvent = {
  type?: string;
  sessionID?: string;
  part?: Record<string, unknown>;
};

export function toolFunctionId(name: string): string {
  return name.startsWith('mcp__')
    ? name.replace(/^mcp__/, '').replace(/__/g, '::')
    : `opencode::${name}`;
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
    provider: 'opencode',
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

/** Normalize a tool result (string | object) to wire content blocks. */
export function mapToolOutput(raw: unknown): ContentBlock[] {
  if (typeof raw === 'string') return [{ type: 'text', text: raw }];
  if (raw == null) return [];
  return [{ type: 'text', text: JSON.stringify(raw) }];
}

/** Sum OpenCode step_finish token counts onto the wire Usage shape. */
export function addUsage(acc: Usage | null, tokens: unknown): Usage {
  const base: Usage = acc ?? {
    input_tokens: 0,
    output_tokens: 0,
    cache_read_tokens: 0,
    cache_write_tokens: 0,
    reasoning_tokens: 0,
  };
  if (typeof tokens !== 'object' || tokens === null) return base;
  const t = tokens as {
    input?: number;
    output?: number;
    reasoning?: number;
    cache?: { read?: number; write?: number };
  };
  return {
    input_tokens: base.input_tokens + (t.input ?? 0),
    output_tokens: base.output_tokens + (t.output ?? 0),
    reasoning_tokens: (base.reasoning_tokens ?? 0) + (t.reasoning ?? 0),
    cache_read_tokens: (base.cache_read_tokens ?? 0) + (t.cache?.read ?? 0),
    cache_write_tokens: (base.cache_write_tokens ?? 0) + (t.cache?.write ?? 0),
  };
}
