/**
 * Anthropic SSE stream parser + state machine. Mirrors
 * `provider-anthropic/src/lib.rs::{handle_sse_event, build_partial,
 * build_final, build_content, merge_usage, map_stop_reason}` and the
 * surrounding stream loop.
 *
 * Consumes `data: {…}` SSE frames, threads them through the partial
 * state, and yields `AssistantMessageEvent` values for the caller.
 */

import { type AssistantMessage, emptyAssistant } from '../types/agent-message.js';
import type { ContentBlock, TextContent } from '../types/content.js';
import type { AssistantMessageEvent, ErrorKind, StopReason, Usage } from '../types/stream-event.js';
import { decodeToolName } from './wire-messages.js';

type PartialFunctionCall = { id: string; function_id: string; args_json: string };

export type PartialState = {
  text_blocks: string[];
  function_calls: PartialFunctionCall[];
  usage: Usage;
  stop_reason: StopReason;
  error_message: string | null;
};

export function emptyPartial(): PartialState {
  return {
    text_blocks: [],
    function_calls: [],
    usage: { input: 0, output: 0, cache_read: 0, cache_write: 0 },
    stop_reason: 'end',
    error_message: null,
  };
}

function buildContent(state: PartialState): ContentBlock[] {
  const out: ContentBlock[] = [];
  for (const t of state.text_blocks) {
    if (t.length > 0) {
      const tc: TextContent = { type: 'text', text: t };
      out.push(tc);
    }
  }
  for (const tc of state.function_calls) {
    let args: unknown = {};
    if (tc.args_json.length > 0) {
      try {
        args = JSON.parse(tc.args_json);
      } catch {
        args = null;
      }
    }
    out.push({
      type: 'function_call',
      id: tc.id,
      function_id: tc.function_id,
      arguments: args,
    });
  }
  return out;
}

export function buildPartial(state: PartialState, model: string): AssistantMessage {
  return {
    role: 'assistant',
    content: buildContent(state),
    stop_reason: state.stop_reason,
    error_message: state.error_message,
    error_kind: null,
    usage: state.usage,
    model,
    provider: 'anthropic',
    timestamp: Date.now(),
  };
}

export function buildFinal(state: PartialState, model: string): AssistantMessage {
  return buildPartial(state, model);
}

export function mapStopReason(s: string): StopReason {
  switch (s) {
    case 'end_turn':
      return 'end';
    case 'max_tokens':
      return 'length';
    case 'tool_use':
      return 'function_call';
    case 'stop_sequence':
      return 'end';
    default:
      return 'end';
  }
}

export function mergeUsage(usage: Record<string, unknown>, into: Usage): void {
  const num = (k: string) => (typeof usage[k] === 'number' ? (usage[k] as number) : 0);
  into.input = (into.input ?? 0) + num('input_tokens');
  into.output = (into.output ?? 0) + num('output_tokens');
  into.cache_read = (into.cache_read ?? 0) + num('cache_read_input_tokens');
  into.cache_write = (into.cache_write ?? 0) + num('cache_creation_input_tokens');
}

/** Process a single SSE event block into 0+ AssistantMessageEvents. */
export function handleSseEvent(
  block: string,
  state: PartialState,
  model: string,
): AssistantMessageEvent[] {
  let dataLine: string | null = null;
  for (const line of block.split('\n')) {
    if (line.startsWith('data: ')) dataLine = line.slice('data: '.length);
  }
  if (!dataLine) return [];
  let parsed: Record<string, unknown>;
  try {
    parsed = JSON.parse(dataLine) as Record<string, unknown>;
  } catch {
    return [];
  }
  const eventType = typeof parsed.type === 'string' ? parsed.type : null;
  if (!eventType) return [];
  const events: AssistantMessageEvent[] = [];
  switch (eventType) {
    case 'message_start': {
      const u = (parsed.message as Record<string, unknown> | undefined)?.usage;
      if (u && typeof u === 'object') mergeUsage(u as Record<string, unknown>, state.usage);
      break;
    }
    case 'content_block_start': {
      const cb = parsed.content_block as Record<string, unknown> | undefined;
      const blockType = typeof cb?.type === 'string' ? cb.type : '';
      if (blockType === 'text') {
        state.text_blocks.push('');
        events.push({ type: 'text_start', partial: buildPartial(state, model) });
      } else if (blockType === 'tool_use') {
        const id = typeof cb?.id === 'string' ? cb.id : '';
        const name = typeof cb?.name === 'string' ? decodeToolName(cb.name) : '';
        state.function_calls.push({ id, function_id: name, args_json: '' });
        events.push({
          type: 'functioncall_start',
          partial: buildPartial(state, model),
        });
      }
      break;
    }
    case 'content_block_delta': {
      const delta = parsed.delta as Record<string, unknown> | undefined;
      const dt = typeof delta?.type === 'string' ? delta.type : '';
      if (dt === 'text_delta') {
        const text = typeof delta?.text === 'string' ? delta.text : '';
        const last = state.text_blocks[state.text_blocks.length - 1];
        if (last !== undefined) {
          state.text_blocks[state.text_blocks.length - 1] = last + text;
        }
        events.push({
          type: 'text_delta',
          partial: buildPartial(state, model),
          delta: text,
        });
      } else if (dt === 'input_json_delta') {
        const json = typeof delta?.partial_json === 'string' ? delta.partial_json : '';
        const last = state.function_calls[state.function_calls.length - 1];
        if (last) last.args_json += json;
        events.push({
          type: 'functioncall_delta',
          partial: buildPartial(state, model),
          delta: json,
        });
      }
      break;
    }
    case 'content_block_stop': {
      events.push({ type: 'text_end', partial: buildPartial(state, model) });
      break;
    }
    case 'message_delta': {
      const d = parsed.delta as Record<string, unknown> | undefined;
      const sr = typeof d?.stop_reason === 'string' ? d.stop_reason : null;
      if (sr) state.stop_reason = mapStopReason(sr);
      const u = parsed.usage as Record<string, unknown> | undefined;
      if (u) mergeUsage(u, state.usage);
      break;
    }
    case 'message_stop': {
      events.push({
        type: 'stop',
        stop_reason: state.stop_reason,
        error_message: state.error_message ?? undefined,
      });
      break;
    }
  }
  return events;
}

export function syntheticErrorEvent(
  message: string,
  model: string,
  error_kind: ErrorKind = 'transient',
): AssistantMessageEvent {
  const final: AssistantMessage = {
    ...emptyAssistant('anthropic', model),
    content: [{ type: 'text', text: message }],
    stop_reason: 'error',
    error_message: message,
    error_kind,
  };
  return { type: 'error', error: final };
}

export function classifyAnthropicError(message: string, status?: number): ErrorKind {
  if (status === 401 || status === 403) return 'auth_expired';
  if (status === 429) return 'rate_limited';
  if (status && status >= 500) return 'transient';
  if (/context|too large|too many tokens/i.test(message)) return 'context_overflow';
  return 'permanent';
}
