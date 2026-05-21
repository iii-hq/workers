// Kept separate from provider-openai so Moonshot-specific extensions
// (e.g. partial_mode) can land without coupling the two providers.

import type { AssistantMessage } from '../types/agent-message.js';
import type { ContentBlock } from '../types/content.js';
import type { AssistantMessageEvent, ErrorKind, StopReason, Usage } from '../types/stream-event.js';

type PartialToolCall = { id: string; function_id: string; args_json: string };

export type PartialState = {
  text: string;
  tool_calls: PartialToolCall[];
  usage: Usage;
  stop_reason: StopReason;
};

export function emptyPartial(): PartialState {
  return {
    text: '',
    tool_calls: [],
    usage: { input: 0, output: 0, cache_read: 0, cache_write: 0 },
    stop_reason: 'end',
  };
}

function buildContent(state: PartialState): ContentBlock[] {
  const out: ContentBlock[] = [];
  if (state.text.length > 0) out.push({ type: 'text', text: state.text });
  for (const tc of state.tool_calls) {
    if (tc.function_id.length === 0) continue;
    let args: unknown = {};
    if (tc.args_json.length > 0) {
      try {
        args = JSON.parse(tc.args_json);
      } catch {
        args = null;
      }
    }
    out.push({ type: 'function_call', id: tc.id, function_id: tc.function_id, arguments: args });
  }
  return out;
}

export function buildPartial(
  state: PartialState,
  model: string,
  provider: string,
): AssistantMessage {
  return {
    role: 'assistant',
    content: buildContent(state),
    stop_reason: state.stop_reason,
    error_message: null,
    error_kind: null,
    usage: state.usage,
    model,
    provider,
    timestamp: Date.now(),
  };
}

export function buildFinal(state: PartialState, model: string, provider: string): AssistantMessage {
  return buildPartial(state, model, provider);
}

export function mapFinishReason(s: string): StopReason {
  if (s === 'stop') return 'end';
  if (s === 'length') return 'length';
  if (s === 'tool_calls' || s === 'function_call') return 'function_call';
  return 'end';
}

export function mergeUsage(usage: Record<string, unknown>, into: Usage): void {
  const num = (k: string) => (typeof usage[k] === 'number' ? (usage[k] as number) : 0);
  into.input = (into.input ?? 0) + num('prompt_tokens') + num('input_tokens');
  into.output = (into.output ?? 0) + num('completion_tokens') + num('output_tokens');
  for (const parent of ['prompt_tokens_details', 'input_tokens_details']) {
    const d = usage[parent] as Record<string, unknown> | undefined;
    if (d && typeof d.cached_tokens === 'number') {
      into.cache_read = (into.cache_read ?? 0) + d.cached_tokens;
    }
  }
}

export function classifyKimiError(message: string, status?: number): ErrorKind {
  if (status === 401 || status === 403) return 'auth_expired';
  if (status === 429) return 'rate_limited';
  if (status && status >= 500) return 'transient';
  if (/context length|too many tokens/i.test(message)) return 'context_overflow';
  return 'permanent';
}

export function syntheticErrorEvent(
  message: string,
  model: string,
  provider: string,
  error_kind: ErrorKind = 'transient',
): AssistantMessageEvent {
  const final: AssistantMessage = {
    role: 'assistant',
    content: [{ type: 'text', text: message }],
    stop_reason: 'error',
    error_message: message,
    error_kind,
    usage: null,
    model,
    provider,
    timestamp: Date.now(),
  };
  return { type: 'error', error: final };
}

export function handleChunk(
  chunk: Record<string, unknown>,
  state: PartialState,
  model: string,
  provider: string,
): AssistantMessageEvent[] {
  const events: AssistantMessageEvent[] = [];
  const usage = chunk.usage as Record<string, unknown> | undefined;
  if (usage) mergeUsage(usage, state.usage);
  const choices = chunk.choices;
  if (!Array.isArray(choices) || choices.length === 0) return events;
  const choice = choices[0] as Record<string, unknown>;
  const finish = typeof choice.finish_reason === 'string' ? choice.finish_reason : null;
  if (finish) state.stop_reason = mapFinishReason(finish);
  const delta = choice.delta as Record<string, unknown> | undefined;
  if (!delta) return events;

  if (typeof delta.content === 'string' && delta.content.length > 0) {
    if (state.text.length === 0) {
      events.push({ type: 'text_start', partial: buildPartial(state, model, provider) });
    }
    state.text += delta.content;
    events.push({
      type: 'text_delta',
      partial: buildPartial(state, model, provider),
      delta: delta.content,
    });
  }

  const tool_calls = delta.tool_calls;
  if (Array.isArray(tool_calls)) {
    for (const tc of tool_calls) {
      if (!tc || typeof tc !== 'object') continue;
      const tcObj = tc as Record<string, unknown>;
      const index = typeof tcObj.index === 'number' ? tcObj.index : 0;
      while (state.tool_calls.length <= index) {
        state.tool_calls.push({ id: '', function_id: '', args_json: '' });
      }
      const entry = state.tool_calls[index];
      if (!entry) continue;
      if (typeof tcObj.id === 'string' && tcObj.id.length > 0) entry.id = tcObj.id;
      const fn = tcObj.function as Record<string, unknown> | undefined;
      if (fn) {
        if (typeof fn.name === 'string' && fn.name.length > 0) entry.function_id = fn.name;
        if (typeof fn.arguments === 'string') {
          entry.args_json += fn.arguments;
          events.push({
            type: 'functioncall_delta',
            partial: buildPartial(state, model, provider),
            delta: fn.arguments,
          });
        }
      }
    }
  }
  return events;
}
