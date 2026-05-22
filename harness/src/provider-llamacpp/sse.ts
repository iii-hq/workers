// llama-server SSE handling. Kept separate from the other OpenAI-compat
// providers so llama.cpp-specific quirks (jinja templating modes,
// reasoning-format toggles) can land without coupling the providers.
//
// Compared to provider-lmstudio/sse.ts this file is intentionally
// thinner: there is no "load failure" path because llama-server runs
// exactly one model loaded at process start (via `-m model.gguf`), so
// there is no `provider::llamacpp::load_model` to point the user at if
// classification fails. Errors classify as transient/auth_expired/
// rate_limited/context_overflow/permanent based on HTTP status alone.

import type { AssistantMessage } from '../types/agent-message.js';
import type { ContentBlock } from '../types/content.js';
import type { AssistantMessageEvent, ErrorKind, StopReason, Usage } from '../types/stream-event.js';

type PartialToolCall = { id: string; function_id: string; args_json: string };

export type PartialState = {
  text: string;
  /**
   * Accumulated reasoning content from thinking-mode models served by
   * llama-server with `--jinja --reasoning-format deepseek` (or
   * equivalent). Stream emits these as `delta.reasoning_content`,
   * matching the LM Studio / Moonshot convention. Persisted as a
   * `thinking` ContentBlock so subsequent requests can echo it back
   * via `reasoning_content` — required by some templates when
   * thinking is enabled and the assistant turn carries tool_calls.
   */
  reasoning_text: string;
  tool_calls: PartialToolCall[];
  usage: Usage;
  stop_reason: StopReason;
  /**
   * Set true the first time we observe a chunk carrying a non-null
   * `finish_reason`. Used by stream.ts to distinguish a legitimate
   * stream end from an abrupt EOF before the server finished.
   */
  saw_finish_reason: boolean;
};

export function emptyPartial(): PartialState {
  return {
    text: '',
    reasoning_text: '',
    tool_calls: [],
    usage: { input: 0, output: 0, cache_read: 0, cache_write: 0 },
    stop_reason: 'end',
    saw_finish_reason: false,
  };
}

function buildContent(state: PartialState): ContentBlock[] {
  const out: ContentBlock[] = [];
  if (state.reasoning_text.length > 0) {
    out.push({ type: 'thinking', text: state.reasoning_text });
  }
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

export function classifyLlamacppError(message: string, status?: number): ErrorKind {
  if (status === 401 || status === 403) return 'auth_expired';
  if (status === 429) return 'rate_limited';
  if (status && status >= 500) return 'transient';
  if (/context length|too many tokens|n_ctx|context window/i.test(message)) {
    return 'context_overflow';
  }
  return 'permanent';
}

export function syntheticErrorEvent(
  message: string,
  model: string,
  provider: string,
  error_kind: ErrorKind = 'transient',
): AssistantMessageEvent {
  // Carry the message ONLY in `error_message` — same security posture
  // as provider-lmstudio (see SECURITY note there). Never inject as a
  // `text` ContentBlock that would be re-fed as trusted context.
  const final: AssistantMessage = {
    role: 'assistant',
    content: [],
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

/**
 * Extract a human-readable error message from an SSE error chunk.
 * llama-server (and other OpenAI-compatible servers) send a JSON chunk
 * shaped `{"error": {"message": "...", ...}}` or `{"error": "..."}`
 * when generation fails mid-stream after the HTTP 200 was already
 * committed. Returns null when the chunk has no error.
 */
export function extractErrorMessage(chunk: Record<string, unknown>): string | null {
  const err = chunk.error;
  if (!err) return null;
  if (typeof err === 'string' && err.length > 0) return err;
  if (typeof err === 'object') {
    const obj = err as Record<string, unknown>;
    if (typeof obj.message === 'string' && obj.message.length > 0) return obj.message;
    if (typeof obj.error_message === 'string' && obj.error_message.length > 0) {
      return obj.error_message;
    }
    if (typeof obj.detail === 'string' && obj.detail.length > 0) return obj.detail;
  }
  return null;
}

export function handleChunk(
  chunk: Record<string, unknown>,
  state: PartialState,
  model: string,
  provider: string,
): AssistantMessageEvent[] {
  const events: AssistantMessageEvent[] = [];

  const errMsg = extractErrorMessage(chunk);
  if (errMsg) {
    state.stop_reason = 'error';
    state.saw_finish_reason = true; // prevent the EOF-without-finish guard
    return [syntheticErrorEvent(errMsg, model, provider, classifyLlamacppError(errMsg))];
  }

  const usage = chunk.usage as Record<string, unknown> | undefined;
  if (usage) mergeUsage(usage, state.usage);
  const choices = chunk.choices;
  if (!Array.isArray(choices) || choices.length === 0) return events;
  const choice = choices[0] as Record<string, unknown>;
  const finish = typeof choice.finish_reason === 'string' ? choice.finish_reason : null;
  if (finish) {
    state.stop_reason = mapFinishReason(finish);
    state.saw_finish_reason = true;
  }
  const delta = choice.delta as Record<string, unknown> | undefined;
  if (!delta) return events;

  if (typeof delta.reasoning_content === 'string' && delta.reasoning_content.length > 0) {
    if (state.reasoning_text.length === 0) {
      events.push({ type: 'thinking_start', partial: buildPartial(state, model, provider) });
    }
    state.reasoning_text += delta.reasoning_content;
    events.push({
      type: 'thinking_delta',
      partial: buildPartial(state, model, provider),
      delta: delta.reasoning_content,
    });
  }

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
      const rawIndex = typeof tcObj.index === 'number' ? tcObj.index : 0;
      // DoS guard against attacker-controlled tool-call indices —
      // same rationale as provider-lmstudio/sse.ts.
      if (
        !Number.isInteger(rawIndex) ||
        rawIndex < 0 ||
        rawIndex > 256
      ) {
        continue;
      }
      const index = rawIndex;
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
