/**
 * Anthropic SSE stream parser + state machine. Mirrors
 * `provider-anthropic/src/lib.rs::{handle_sse_event, build_partial,
 * build_final, build_content, merge_usage, map_stop_reason}` and the
 * surrounding stream loop.
 *
 * Consumes `data: {…}` SSE frames, threads them through the partial
 * state, and yields `AssistantMessageEvent` values for the caller.
 */

import { logger } from '../runtime/otel.js';
import { type AssistantMessage, emptyAssistant } from '../types/agent-message.js';
import type { ContentBlock, TextContent, ThinkingContent } from '../types/content.js';
import type { AssistantMessageEvent, ErrorKind, StopReason, Usage } from '../types/stream-event.js';
import { decodeToolName } from './wire-messages.js';

type PartialFunctionCall = { id: string; function_id: string; args_json: string };

type PartialThinking = { text: string; signature?: string };

type BlockKind = 'text' | 'tool_use' | 'thinking';

type OpenBlockKind = BlockKind | null;

export type PartialState = {
  text_blocks: string[];
  thinking_blocks: PartialThinking[];
  function_calls: PartialFunctionCall[];
  /**
   * Wire arrival order of content blocks ({kind, index-within-kind-array}).
   * Replayed turns must keep thinking blocks in their original position
   * relative to tool_use, so `buildContent` reconstructs in this order.
   */
  block_order: Array<{ kind: BlockKind; idx: number }>;
  /** Kind of the currently open content block so `content_block_stop` emits the matching end event. */
  open_block: OpenBlockKind;
  usage: Usage;
  stop_reason: StopReason;
  error_message: string | null;
};

export function emptyPartial(): PartialState {
  return {
    text_blocks: [],
    thinking_blocks: [],
    function_calls: [],
    block_order: [],
    open_block: null,
    usage: { input: 0, output: 0, cache_read: 0, cache_write: 0 },
    stop_reason: 'end',
    error_message: null,
  };
}

function pushBlockContent(out: ContentBlock[], state: PartialState, kind: BlockKind, idx: number) {
  if (kind === 'thinking') {
    const th = state.thinking_blocks[idx];
    if (th && th.text.length > 0) {
      const tc: ThinkingContent = { type: 'thinking', text: th.text };
      if (th.signature) tc.signature = th.signature;
      out.push(tc);
    }
    return;
  }
  if (kind === 'text') {
    const t = state.text_blocks[idx];
    if (t !== undefined && t.length > 0) {
      const tc: TextContent = { type: 'text', text: t };
      out.push(tc);
    }
    return;
  }
  const tc = state.function_calls[idx];
  if (!tc) return;
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

function buildContent(state: PartialState): ContentBlock[] {
  const out: ContentBlock[] = [];
  // Wire arrival order first; block indices not tracked in block_order
  // (state built directly in tests) are appended grouped afterwards.
  const seen = {
    text: new Set<number>(),
    thinking: new Set<number>(),
    tool_use: new Set<number>(),
  };
  for (const e of state.block_order) {
    if (seen[e.kind].has(e.idx)) continue;
    seen[e.kind].add(e.idx);
    pushBlockContent(out, state, e.kind, e.idx);
  }
  for (let i = 0; i < state.thinking_blocks.length; i++) {
    if (!seen.thinking.has(i)) pushBlockContent(out, state, 'thinking', i);
  }
  for (let i = 0; i < state.text_blocks.length; i++) {
    if (!seen.text.has(i)) pushBlockContent(out, state, 'text', i);
  }
  for (let i = 0; i < state.function_calls.length; i++) {
    if (!seen.tool_use.has(i)) pushBlockContent(out, state, 'tool_use', i);
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
        state.block_order.push({ kind: 'text', idx: state.text_blocks.length });
        state.text_blocks.push('');
        state.open_block = 'text';
        events.push({ type: 'text_start', partial: buildPartial(state, model) });
      } else if (blockType === 'tool_use') {
        const id = typeof cb?.id === 'string' ? cb.id : '';
        const name = typeof cb?.name === 'string' ? decodeToolName(cb.name) : '';
        state.block_order.push({ kind: 'tool_use', idx: state.function_calls.length });
        state.function_calls.push({ id, function_id: name, args_json: '' });
        state.open_block = 'tool_use';
        events.push({
          type: 'functioncall_start',
          partial: buildPartial(state, model),
        });
      } else if (blockType === 'thinking' || blockType === 'redacted_thinking') {
        // Redacted thinking is opaque and not persisted/round-tripped
        // (needs a ContentBlock extension — follow-up); logged because the
        // API expects it back during tool use.
        if (blockType === 'redacted_thinking') {
          logger.warn('anthropic redacted_thinking block received; not persisted/round-tripped', {
            model,
          });
        }
        state.block_order.push({ kind: 'thinking', idx: state.thinking_blocks.length });
        state.thinking_blocks.push({ text: '' });
        state.open_block = 'thinking';
        events.push({ type: 'thinking_start', partial: buildPartial(state, model) });
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
      } else if (dt === 'thinking_delta') {
        const text = typeof delta?.thinking === 'string' ? delta.thinking : '';
        const last = state.thinking_blocks[state.thinking_blocks.length - 1];
        if (last) last.text += text;
        events.push({
          type: 'thinking_delta',
          partial: buildPartial(state, model),
          delta: text,
        });
      } else if (dt === 'signature_delta') {
        const sig = typeof delta?.signature === 'string' ? delta.signature : '';
        const last = state.thinking_blocks[state.thinking_blocks.length - 1];
        if (last && sig) last.signature = (last.signature ?? '') + sig;
      }
      break;
    }
    case 'content_block_stop': {
      // Emit the end event matching the open block. Default to text_end for
      // unknown/untracked blocks (preserves pre-thinking behavior).
      const kind = state.open_block;
      state.open_block = null;
      if (kind === 'thinking') {
        events.push({ type: 'thinking_end', partial: buildPartial(state, model) });
      } else if (kind === 'tool_use') {
        events.push({ type: 'functioncall_end', partial: buildPartial(state, model) });
      } else {
        events.push({ type: 'text_end', partial: buildPartial(state, model) });
      }
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
